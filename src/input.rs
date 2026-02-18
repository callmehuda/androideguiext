use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::sync::mpsc;
use std::thread;

use tracing::{debug, info, warn};

// Linux input event structs (from <linux/input.h>)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    event_type: u16,
    code: u16,
    value: i32,
}

// Event types
const EV_SYN: u16 = 0x00;
const EV_ABS: u16 = 0x03;
const EV_KEY: u16 = 0x01;

// ABS codes for multitouch Protocol B
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;

// Single touch fallback (Protocol A)
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

// SYN
const SYN_REPORT: u16 = 0x00;

// KEY codes
const BTN_TOUCH: u16 = 0x14a;

/// Slot state for multitouch Protocol B.
/// `prev_tracking_id` lets us detect finger-down (Start) vs ongoing move.
#[derive(Debug, Clone)]
struct SlotState {
    /// Current tracking id. -1 means the slot is free (finger lifted).
    tracking_id: i32,
    /// The tracking id we saw in the *previous* SYN_REPORT frame.
    /// When tracking_id transitions from -1 → >=0 we emit TouchPhase::Start.
    prev_tracking_id: i32,
    x: i32,
    y: i32,
    /// Whether x/y have been set at least once (so we don't send garbage coords).
    has_pos: bool,
}

impl Default for SlotState {
    fn default() -> Self {
        Self {
            tracking_id: -1,
            prev_tracking_id: -1,
            x: 0,
            y: 0,
            has_pos: false,
        }
    }
}

/// Per-device coordinate mapper.
///
/// Android touchscreen sensors are often physically oriented in landscape
/// (the panel is wider than tall), even on a portrait phone. The driver
/// reports raw ABS_MT_POSITION_X along the **long** axis of the sensor and
/// ABS_MT_POSITION_Y along the **short** axis.  When the phone is held in
/// portrait mode the long axis of the sensor is vertical on screen, so
/// sensor-X → screen-Y and sensor-Y → screen-X.
///
/// We detect this by comparing the sensor aspect ratio (from ioctl ranges)
/// Maps raw sensor coordinates to egui screen coordinates.
///
/// A touchscreen sensor has a fixed physical orientation that never changes.
/// Android reports the current display rotation (0/1/2/3) separately.
/// We use BOTH the sensor's natural aspect ratio AND the display rotation
/// to deterministically compute the correct swap + flip transform.
///
/// The 8 possible transforms are:
///   swap_xy | flip_x | flip_y
/// We pick the one that makes sensor movement match screen movement.
#[derive(Debug, Clone)]
struct CoordMapper {
    raw_x_min: i32,
    raw_x_max: i32,
    raw_y_min: i32,
    raw_y_max: i32,
    swap_xy: bool,
    flip_x: bool,
    flip_y: bool,
}

impl CoordMapper {
    /// Construct from ioctl axis ranges + Android display rotation.
    ///
    /// `display_rotation`: value from Android WindowManager
    ///   0 = ROTATION_0   (natural/portrait)
    ///   1 = ROTATION_90  (landscape, rotated 90° clockwise)
    ///   2 = ROTATION_180 (upside-down portrait)
    ///   3 = ROTATION_270 (landscape, rotated 270° clockwise)
    fn new(
        ioctl_x: (i32, i32),
        ioctl_y: (i32, i32),
        screen_w: f32,
        screen_h: f32,
        display_rotation: i32,
    ) -> Self {
        let sensor_x_span = (ioctl_x.1 - ioctl_x.0).max(1) as f32;
        let sensor_y_span = (ioctl_y.1 - ioctl_y.0).max(1) as f32;

        // Is the sensor physically landscape-oriented?
        // (its X axis is longer than its Y axis)
        let sensor_is_landscape = sensor_x_span < sensor_y_span;

        // Is the screen currently showing in landscape?
        let screen_is_landscape = screen_w < screen_h;

        // Step 1: Do we need to swap sensor X↔Y axes?
        // We need a swap when sensor orientation differs from screen orientation.
        // e.g. sensor is landscape but screen is portrait → swap.
        let swap_xy = sensor_is_landscape != screen_is_landscape;

        // Step 2: After potential swap, determine flips.
        // Android rotation tells us the clockwise degrees the screen has been rotated
        // from its natural orientation. We use this to pick the right flip.
        //
        // Convention after swap:
        //   nx = 0..1 where 0=left, 1=right
        //   ny = 0..1 where 0=top,  1=bottom
        //
        // For each rotation value, empirically:
        //   ROTATION_0   (portrait natural)    : no flip
        //   ROTATION_90  (landscape CW)        : flip_y
        //   ROTATION_180 (portrait upside-down): flip_x + flip_y
        //   ROTATION_270 (landscape CCW)       : flip_x
        //
        // If sensor is landscape-mounted (swap=true), the base orientation
        // is different so we invert the flip logic.
        let (flip_x, flip_y) = match display_rotation {
            0 => (false, false),
            1 => (false, true),
            2 => (true,  true),
            3 => (true,  false),
            _ => (false, false),
        };

        // When we swap axes we also have to invert which flip applies to which axis,
        // because after swap what was sensor-X is now occupying the Y screen slot.
        // But since we swap before flipping, the flip flags already refer to the
        // post-swap (screen-aligned) axes, so no extra inversion needed.

        info!(
            "CoordMapper: sensor_span=({:.0}x{:.0}) sensor_landscape={}              screen=({:.0}x{:.0}) screen_landscape={} rotation={}              → swap={} flip_x={} flip_y={}",
            sensor_x_span, sensor_y_span, sensor_is_landscape,
            screen_w, screen_h, screen_is_landscape, display_rotation,
            swap_xy, flip_x, flip_y
        );

        Self {
            raw_x_min: ioctl_x.0,
            raw_x_max: ioctl_x.1,
            raw_y_min: ioctl_y.0,
            raw_y_max: ioctl_y.1,
            swap_xy,
            flip_x,
            flip_y,
        }
    }

    fn to_screen(&self, raw_x: i32, raw_y: i32, screen_w: f32, screen_h: f32) -> egui::Pos2 {
        let x_span = (self.raw_x_max - self.raw_x_min).max(1) as f32;
        let y_span = (self.raw_y_max - self.raw_y_min).max(1) as f32;

        // Normalize to 0..1 in sensor space
        let mut nx = (raw_x - self.raw_x_min) as f32 / x_span;
        let mut ny = (raw_y - self.raw_y_min) as f32 / y_span;

        // 1. Swap axes if sensor orientation differs from screen orientation
        if self.swap_xy { std::mem::swap(&mut nx, &mut ny); }

        // 2. Flip as needed for display rotation
        if self.flip_x { nx = 1.0 - nx; }
        if self.flip_y { ny = 1.0 - ny; }

        egui::pos2(nx * screen_w, ny * screen_h)
    }
}

/// Read axis ranges from the kernel via ioctl EVIOCGABS.
/// Returns (min, max) for a given abs axis code, or None on failure.
fn read_abs_range(fd: i32, axis: u16) -> Option<(i32, i32)> {
    // struct input_absinfo { value, minimum, maximum, fuzz, flat, resolution }
    #[repr(C)]
    struct AbsInfo {
        value: i32,
        minimum: i32,
        maximum: i32,
        fuzz: i32,
        flat: i32,
        resolution: i32,
    }
    let mut info = AbsInfo {
        value: 0,
        minimum: 0,
        maximum: 0,
        fuzz: 0,
        flat: 0,
        resolution: 0,
    };
    // EVIOCGABS(axis) = _IOR('E', 0x40 + axis, struct input_absinfo)
    // _IOR(type, nr, size) = (2<<30)|(sizeof<<16)|(type<<8)|nr
    // Cast via u32 then as i32 (wrapping) — libc::ioctl takes Ioctl = i32 on Android.
    let size = std::mem::size_of::<AbsInfo>() as u32;
    let ioctl_nr = ((2u32 << 30) | (size << 16) | ((b'E' as u32) << 8) | (0x40u32 + axis as u32)) as i32;
    let ret = unsafe { libc::ioctl(fd, ioctl_nr, &mut info as *mut _) };
    if ret == 0 && info.maximum > info.minimum {
        Some((info.minimum, info.maximum))
    } else {
        None
    }
}

/// Find touchscreen input devices from /proc/bus/input/devices.
/// Returns a list of /dev/input/eventX paths.
fn find_touch_devices() -> Vec<String> {
    let mut devices = Vec::new();

    if let Ok(data) = fs::read_to_string("/proc/bus/input/devices") {
        let mut current_handlers: Vec<String> = Vec::new();
        let mut has_abs = false;
        let mut is_touch_name = false;

        for line in data.lines() {
            if line.starts_with("N: Name=") {
                let name = line.to_lowercase();
                is_touch_name = name.contains("touch")
                    || name.contains("ts")
                    || name.contains("finger");
                has_abs = false;
                current_handlers.clear();
            } else if line.starts_with("B: ABS=") {
                // If ABS_MT_POSITION_X (bit 0x35 = 53) is set, it's a touchscreen.
                // The ABS bitmap is hex, space-separated from MSB chunks.
                // bit 53 is in the second hex word (bits 64-32 range).
                // We just check if the device advertises *any* ABS capability.
                has_abs = true;
            } else if line.starts_with("H: Handlers=") {
                current_handlers.clear();
                for part in line.split_whitespace() {
                    if part.starts_with("event") {
                        current_handlers.push(format!("/dev/input/{}", part));
                    }
                }
            } else if line.is_empty() {
                if has_abs && (is_touch_name || !current_handlers.is_empty()) {
                    for h in &current_handlers {
                        info!("Found touch candidate: {}", h);
                        devices.push(h.clone());
                    }
                }
                has_abs = false;
                is_touch_name = false;
                current_handlers.clear();
            }
        }
    }

    // Fallback: open all event devices
    if devices.is_empty() {
        for i in 0..20 {
            let path = format!("/dev/input/event{}", i);
            if std::path::Path::new(&path).exists() {
                devices.push(path);
            }
        }
    }

    devices
}

/// Start a background thread reading raw Linux touch events.
/// Emits properly sequenced egui events (Touch Start/Move/End + PointerButton + PointerMoved/Gone).
pub fn start_input_thread(
    screen_width: f32,
    screen_height: f32,
    display_rotation: i32,
) -> mpsc::Receiver<Vec<egui::Event>> {
    let (tx, rx) = mpsc::channel::<Vec<egui::Event>>();

    thread::Builder::new()
        .name("input-reader".into())
        .spawn(move || {
            let devices = find_touch_devices();
            if devices.is_empty() {
                warn!("No input devices found.");
                return;
            }

            info!("Opening input devices: {:?}", devices);

            let mut files: Vec<File> = devices
                .iter()
                .filter_map(|path| {
                    File::open(path)
                        .map_err(|e| warn!("Cannot open {}: {}", path, e))
                        .ok()
                })
                .collect();

            if files.is_empty() {
                warn!("Could not open any input devices.");
                return;
            }

            let num_devices = files.len();

            // Per-device multitouch slot state (Protocol B, up to 10 fingers)
            const MAX_SLOTS: usize = 10;
            let mut slots: Vec<Vec<SlotState>> = (0..num_devices)
                .map(|_| (0..MAX_SLOTS).map(|_| SlotState::default()).collect())
                .collect();
            let mut current_slot: Vec<usize> = vec![0; num_devices];

            // Per-device axis ranges (seeded from ioctl, refined live from events)
            let mut ioctl_range_x: Vec<(i32, i32)> = vec![(0, 32767); num_devices];
            let mut ioctl_range_y: Vec<(i32, i32)> = vec![(0, 32767); num_devices];

            // Single-touch (Protocol A) fallback state
            let mut st_x: Vec<i32> = vec![0; num_devices];
            let mut st_y: Vec<i32> = vec![0; num_devices];
            // 0=up, 1=down (tracking down state across frames)
            let mut st_down: Vec<bool> = vec![false; num_devices];
            let mut st_was_down: Vec<bool> = vec![false; num_devices];

            // Seed axis ranges via ioctl (best-effort; refined live from events)
            for (dev_idx, file) in files.iter().enumerate() {
                let fd = file.as_raw_fd();
                if let Some(r) = read_abs_range(fd, ABS_MT_POSITION_X)
                    .or_else(|| read_abs_range(fd, ABS_X))
                {
                    ioctl_range_x[dev_idx] = r;
                    info!("Device {} ioctl X range: {:?}", dev_idx, r);
                } else {
                    info!("Device {} ioctl X range: unavailable, using default 0..32767", dev_idx);
                }
                if let Some(r) = read_abs_range(fd, ABS_MT_POSITION_Y)
                    .or_else(|| read_abs_range(fd, ABS_Y))
                {
                    ioctl_range_y[dev_idx] = r;
                    info!("Device {} ioctl Y range: {:?}", dev_idx, r);
                } else {
                    info!("Device {} ioctl Y range: unavailable, using default 0..32767", dev_idx);
                }
            }

            // Build CoordMapper per device
            let mappers: Vec<CoordMapper> = (0..num_devices)
                .map(|i| CoordMapper::new(
                    ioctl_range_x[i],
                    ioctl_range_y[i],
                    screen_width,
                    screen_height,
                    display_rotation,
                ))
                .collect();

            // epoll setup
            let epoll_fd = unsafe { libc::epoll_create1(0) };
            if epoll_fd < 0 {
                warn!("epoll_create1 failed");
                return;
            }

            let mut fd_to_dev: HashMap<i32, usize> = HashMap::new();
            for (dev_idx, file) in files.iter().enumerate() {
                let fd = file.as_raw_fd();
                fd_to_dev.insert(fd, dev_idx);
                let mut ev = libc::epoll_event {
                    events: libc::EPOLLIN as u32,
                    u64: fd as u64,
                };
                unsafe {
                    libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut ev);
                }
            }

            let mut epoll_events = vec![libc::epoll_event { events: 0, u64: 0 }; num_devices];
            let event_size = std::mem::size_of::<InputEvent>();
            let mut buf = vec![0u8; event_size];

            info!("Input thread listening for events...");

            loop {
                let nfds = unsafe {
                    libc::epoll_wait(
                        epoll_fd,
                        epoll_events.as_mut_ptr(),
                        num_devices as i32,
                        50,
                    )
                };

                if nfds <= 0 {
                    continue;
                }

                for i in 0..nfds as usize {
                    let fd = epoll_events[i].u64 as i32;
                    let dev_idx = match fd_to_dev.get(&fd) {
                        Some(&idx) => idx,
                        None => continue,
                    };

                    // Read exactly one event struct at a time
                    let file = &mut files[dev_idx];
                    let n = match file.read(&mut buf) {
                        Ok(n) => n,
                        Err(_) => continue,
                    };
                    if n < event_size {
                        continue;
                    }

                    let evt: InputEvent =
                        unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const _) };

                    match evt.event_type {
                        EV_ABS => {
                            let slot = current_slot[dev_idx];
                            match evt.code {
                                ABS_MT_SLOT => {
                                    let s = evt.value as usize;
                                    if s < MAX_SLOTS {
                                        current_slot[dev_idx] = s;
                                    }
                                }
                                ABS_MT_TRACKING_ID => {
                                    // Do NOT update prev_tracking_id here.
                                    // We update it only after SYN_REPORT so we can
                                    // compare before/after per frame.
                                    slots[dev_idx][slot].tracking_id = evt.value;
                                }
                                ABS_MT_POSITION_X => {
                                    slots[dev_idx][slot].x = evt.value;
                                    slots[dev_idx][slot].has_pos = true;
                                }
                                ABS_MT_POSITION_Y => {
                                    slots[dev_idx][slot].y = evt.value;
                                }
                                ABS_X => {
                                    st_x[dev_idx] = evt.value;
                                }
                                ABS_Y => {
                                    st_y[dev_idx] = evt.value;
                                }
                                _ => {}
                            }
                        }

                        EV_KEY => {
                            if evt.code == BTN_TOUCH {
                                st_down[dev_idx] = evt.value != 0;
                            }
                        }

                        EV_SYN => {
                            if evt.code != SYN_REPORT {
                                continue;
                            }

                            let mut egui_events: Vec<egui::Event> = Vec::new();

                            let normalize = |raw_x: i32, raw_y: i32| -> egui::Pos2 {
                                let pos = mappers[dev_idx].to_screen(raw_x, raw_y, screen_width, screen_height);
                                debug!("raw({},{}) swap={} => screen({:.1},{:.1})",
                                    raw_x, raw_y, mappers[dev_idx].swap_xy, pos.x, pos.y);
                                pos
                            };

                            // ---- Protocol B: multitouch slots ----
                            let mut primary_slot_handled = false;

                            for slot_idx in 0..MAX_SLOTS {
                                let slot = &mut slots[dev_idx][slot_idx];
                                let cur_tid = slot.tracking_id;
                                let prev_tid = slot.prev_tracking_id;

                                let phase = if prev_tid < 0 && cur_tid >= 0 {
                                    // Finger just pressed down → Start
                                    Some(egui::TouchPhase::Start)
                                } else if prev_tid >= 0 && cur_tid < 0 {
                                    // Finger lifted → End
                                    Some(egui::TouchPhase::End)
                                } else if cur_tid >= 0 && slot.has_pos {
                                    // Finger still down and position updated → Move
                                    Some(egui::TouchPhase::Move)
                                } else {
                                    None
                                };

                                if let Some(phase) = phase {
                                    if !slot.has_pos && phase != egui::TouchPhase::End {
                                        // No position yet; skip until we have coords
                                        slot.prev_tracking_id = cur_tid;
                                        continue;
                                    }

                                    let pos = normalize(slot.x, slot.y);

                                    let touch_id =
                                        egui::TouchId::from(dev_idx as u64 * 1000 + slot_idx as u64);

                                    egui_events.push(egui::Event::Touch {
                                        device_id: egui::TouchDeviceId(dev_idx as u64),
                                        id: touch_id,
                                        phase,
                                        pos,
                                        force: Some(1.0),
                                    });

                                    // Primary finger drives the logical pointer so egui
                                    // widgets (buttons, sliders, etc.) respond correctly.
                                    if slot_idx == 0 || !primary_slot_handled {
                                        primary_slot_handled = true;
                                        match phase {
                                            egui::TouchPhase::Start => {
                                                egui_events.push(egui::Event::PointerMoved(pos));
                                                egui_events.push(egui::Event::PointerButton {
                                                    pos,
                                                    button: egui::PointerButton::Primary,
                                                    pressed: true,
                                                    modifiers: egui::Modifiers::NONE,
                                                });
                                            }
                                            egui::TouchPhase::Move => {
                                                egui_events.push(egui::Event::PointerMoved(pos));
                                            }
                                            egui::TouchPhase::End
                                            | egui::TouchPhase::Cancel => {
                                                egui_events.push(egui::Event::PointerButton {
                                                    pos,
                                                    button: egui::PointerButton::Primary,
                                                    pressed: false,
                                                    modifiers: egui::Modifiers::NONE,
                                                });
                                                egui_events.push(egui::Event::PointerGone);
                                            }
                                        }
                                    }
                                }

                                // Commit: update prev_tracking_id and reset dirty flag
                                slot.prev_tracking_id = cur_tid;
                                slot.has_pos = false;
                            }

                            // ---- Protocol A single-touch fallback ----
                            // Only use if no MT events were produced for this device.
                            if !primary_slot_handled {
                                let pos = normalize(st_x[dev_idx], st_y[dev_idx]);
                                let now_down = st_down[dev_idx];
                                let was_down = st_was_down[dev_idx];

                                if now_down && !was_down {
                                    // Finger down
                                    egui_events.push(egui::Event::Touch {
                                        device_id: egui::TouchDeviceId(dev_idx as u64),
                                        id: egui::TouchId::from(dev_idx as u64 * 1000),
                                        phase: egui::TouchPhase::Start,
                                        pos,
                                        force: Some(1.0),
                                    });
                                    egui_events.push(egui::Event::PointerMoved(pos));
                                    egui_events.push(egui::Event::PointerButton {
                                        pos,
                                        button: egui::PointerButton::Primary,
                                        pressed: true,
                                        modifiers: egui::Modifiers::NONE,
                                    });
                                } else if now_down {
                                    // Drag
                                    egui_events.push(egui::Event::Touch {
                                        device_id: egui::TouchDeviceId(dev_idx as u64),
                                        id: egui::TouchId::from(dev_idx as u64 * 1000),
                                        phase: egui::TouchPhase::Move,
                                        pos,
                                        force: Some(1.0),
                                    });
                                    egui_events.push(egui::Event::PointerMoved(pos));
                                } else if !now_down && was_down {
                                    // Finger up
                                    egui_events.push(egui::Event::Touch {
                                        device_id: egui::TouchDeviceId(dev_idx as u64),
                                        id: egui::TouchId::from(dev_idx as u64 * 1000),
                                        phase: egui::TouchPhase::End,
                                        pos,
                                        force: Some(1.0),
                                    });
                                    egui_events.push(egui::Event::PointerButton {
                                        pos,
                                        button: egui::PointerButton::Primary,
                                        pressed: false,
                                        modifiers: egui::Modifiers::NONE,
                                    });
                                    egui_events.push(egui::Event::PointerGone);
                                }

                                st_was_down[dev_idx] = now_down;
                            }

                            if !egui_events.is_empty() {
                                debug!(
                                    "dev={} sending {} events",
                                    dev_idx,
                                    egui_events.len()
                                );
                                let _ = tx.send(egui_events);
                            }
                        }

                        _ => {}
                    }
                }
            }
        })
        .expect("Failed to spawn input thread");

    rx
}
