use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

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

// ABS codes for multitouch
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_PRESSURE: u16 = 0x3a;

// Single touch fallback
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_PRESSURE: u16 = 0x18;

// SYN
const SYN_REPORT: u16 = 0x00;

// KEY codes
const BTN_TOUCH: u16 = 0x14a;

/// A raw touch point from the input system
#[derive(Debug, Clone)]
pub struct TouchPoint {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub phase: TouchPhase,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TouchPhase {
    Start,
    Move,
    End,
}

/// Converts to egui::TouchPhase
impl From<TouchPhase> for egui::TouchPhase {
    fn from(val: TouchPhase) -> Self {
        match val {
            TouchPhase::Start => egui::TouchPhase::Start,
            TouchPhase::Move => egui::TouchPhase::Move,
            TouchPhase::End => egui::TouchPhase::End,
        }
    }
}

/// Slot state for multitouch protocol B
#[derive(Debug, Clone, Default)]
struct SlotState {
    tracking_id: i32, // -1 = released
    x: i32,
    y: i32,
    pressure: i32,
    active: bool,
}

/// Find all touchscreen input devices under /dev/input/
fn find_touch_devices() -> Vec<String> {
    let mut devices = Vec::new();

    // Try reading /proc/bus/input/devices to find touchscreen
    if let Ok(data) = fs::read_to_string("/proc/bus/input/devices") {
        let mut current_handlers = Vec::new();
        let mut is_touch = false;

        for line in data.lines() {
            if line.starts_with("N: Name=") {
                let name = line.to_lowercase();
                is_touch = name.contains("touch")
                    || name.contains("touchscreen")
                    || name.contains("ts")
                    || name.contains("input");
            } else if line.starts_with("H: Handlers=") {
                current_handlers.clear();
                for part in line.split_whitespace() {
                    if part.starts_with("event") {
                        current_handlers.push(format!("/dev/input/{}", part));
                    }
                }
            } else if line.is_empty() {
                if is_touch {
                    for handler in &current_handlers {
                        info!("Found potential touch device: {}", handler);
                        devices.push(handler.clone());
                    }
                }
                is_touch = false;
                current_handlers.clear();
            }
        }
    }

    // Fallback: try common event device paths
    if devices.is_empty() {
        for i in 0..10 {
            let path = format!("/dev/input/event{}", i);
            if std::path::Path::new(&path).exists() {
                devices.push(path);
            }
        }
    }

    devices
}

/// Start a background thread that reads raw Linux touch events and converts them to egui events.
/// Returns a receiver for egui events.
pub fn start_input_thread(
    screen_width: f32,
    screen_height: f32,
) -> mpsc::Receiver<Vec<egui::Event>> {
    let (tx, rx) = mpsc::channel::<Vec<egui::Event>>();
    let start_time = Instant::now();

    thread::Builder::new()
        .name("input-reader".into())
        .spawn(move || {
            let devices = find_touch_devices();
            if devices.is_empty() {
                warn!("No input devices found. Touch input will not work.");
                return;
            }

            info!("Opening input devices: {:?}", devices);

            // Open all candidate devices
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

            // For each device, maintain per-device slot state
            let num_devices = files.len();
            let mut slots: Vec<Vec<SlotState>> = (0..num_devices)
                .map(|_| {
                    (0..10)
                        .map(|_| SlotState {
                            tracking_id: -1,
                            ..Default::default()
                        })
                        .collect()
                })
                .collect();
            let mut current_slots: Vec<usize> = vec![0; num_devices];

            // For single-touch fallback
            let mut single_x: Vec<i32> = vec![0; num_devices];
            let mut single_y: Vec<i32> = vec![0; num_devices];
            let mut single_active: Vec<bool> = vec![false; num_devices];

            // Known max coords per device (we'll auto-detect from first events or assume 32767)
            let mut max_x: Vec<i32> = vec![32767; num_devices];
            let mut max_y: Vec<i32> = vec![32767; num_devices];

            let event_size = std::mem::size_of::<InputEvent>();
            let mut buf = vec![0u8; event_size];

            // Use epoll for non-blocking multi-device reading
            let epoll_fd = unsafe { libc::epoll_create1(0) };
            if epoll_fd < 0 {
                warn!("epoll_create failed");
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

            let mut epoll_events = vec![
                libc::epoll_event { events: 0, u64: 0 };
                num_devices
            ];

            info!("Input thread started. Listening for touch events...");

            loop {
                let nfds = unsafe {
                    libc::epoll_wait(
                        epoll_fd,
                        epoll_events.as_mut_ptr(),
                        num_devices as i32,
                        100, // 100ms timeout
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

                    let file = &mut files[dev_idx];
                    let n = match file.read(&mut buf) {
                        Ok(n) => n,
                        Err(_) => continue,
                    };
                    if n < event_size {
                        continue;
                    }

                    let evt: InputEvent = unsafe { std::ptr::read(buf.as_ptr() as *const _) };

                    match evt.event_type {
                        EV_ABS => {
                            let slot = current_slots[dev_idx];
                            match evt.code {
                                ABS_MT_SLOT => {
                                    current_slots[dev_idx] = evt.value as usize;
                                }
                                ABS_MT_TRACKING_ID => {
                                    slots[dev_idx][slot].tracking_id = evt.value;
                                    if evt.value >= 0 {
                                        slots[dev_idx][slot].active = true;
                                    }
                                }
                                ABS_MT_POSITION_X => {
                                    slots[dev_idx][slot].x = evt.value;
                                    // Auto-detect range
                                    if evt.value > max_x[dev_idx] {
                                        max_x[dev_idx] = evt.value;
                                    }
                                }
                                ABS_MT_POSITION_Y => {
                                    slots[dev_idx][slot].y = evt.value;
                                    if evt.value > max_y[dev_idx] {
                                        max_y[dev_idx] = evt.value;
                                    }
                                }
                                ABS_X => {
                                    single_x[dev_idx] = evt.value;
                                    if evt.value > max_x[dev_idx] {
                                        max_x[dev_idx] = evt.value;
                                    }
                                }
                                ABS_Y => {
                                    single_y[dev_idx] = evt.value;
                                    if evt.value > max_y[dev_idx] {
                                        max_y[dev_idx] = evt.value;
                                    }
                                }
                                _ => {}
                            }
                        }
                        EV_KEY => {
                            if evt.code == BTN_TOUCH {
                                single_active[dev_idx] = evt.value != 0;
                            }
                        }
                        EV_SYN => {
                            if evt.code == SYN_REPORT {
                                let mut egui_events = Vec::new();
                                let mx = max_x[dev_idx].max(1) as f32;
                                let my = max_y[dev_idx].max(1) as f32;

                                // Multitouch Protocol B
                                for (slot_idx, slot) in slots[dev_idx].iter_mut().enumerate() {
                                    if !slot.active {
                                        continue;
                                    }

                                    let screen_x = (slot.x as f32 / mx) * screen_width;
                                    let screen_y = (slot.y as f32 / my) * screen_height;
                                    let pos = egui::pos2(screen_x, screen_y);

                                    let (phase, released) = if slot.tracking_id < 0 {
                                        // Finger released
                                        (egui::TouchPhase::End, true)
                                    } else {
                                        (egui::TouchPhase::Move, false)
                                    };

                                    let touch_id = egui::TouchId::from(
                                        (dev_idx as u64) * 1000 + slot_idx as u64,
                                    );

                                    egui_events.push(egui::Event::Touch {
                                        device_id: egui::TouchDeviceId(dev_idx as u64),
                                        id: touch_id,
                                        phase,
                                        pos,
                                        force: Some(1.0),
                                    });

                                    // Also emit pointer events for primary finger (slot 0)
                                    if slot_idx == 0 {
                                        if released {
                                            egui_events.push(egui::Event::PointerGone);
                                        } else {
                                            egui_events.push(egui::Event::PointerMoved(pos));
                                        }
                                    }

                                    if released {
                                        slot.active = false;
                                        slot.tracking_id = -1;
                                    }
                                }

                                // Single-touch fallback
                                if egui_events.is_empty() && single_active[dev_idx] {
                                    let screen_x =
                                        (single_x[dev_idx] as f32 / mx) * screen_width;
                                    let screen_y =
                                        (single_y[dev_idx] as f32 / my) * screen_height;
                                    let pos = egui::pos2(screen_x, screen_y);

                                    egui_events.push(egui::Event::Touch {
                                        device_id: egui::TouchDeviceId(dev_idx as u64),
                                        id: egui::TouchId::from(0u64),
                                        phase: egui::TouchPhase::Move,
                                        pos,
                                        force: Some(1.0),
                                    });
                                    egui_events.push(egui::Event::PointerMoved(pos));
                                }

                                if !egui_events.is_empty() {
                                    debug!("Sending {} egui events", egui_events.len());
                                    let _ = tx.send(egui_events);
                                }
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
