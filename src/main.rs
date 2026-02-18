use anyhow::Result;
use tracing::info;

use crate::android::runtime::AndroidRuntime;

mod android;
mod bridge;
mod dex;
mod input;
mod jni;
mod renderer;

struct App {
    checkbox_val: bool,
    touch_pos: Option<egui::Pos2>,
    touch_count: u32,
    last_event: String,
}

impl App {
    fn new() -> Self {
        Self {
            checkbox_val: false,
            touch_pos: None,
            touch_count: 0,
            last_event: "none".to_string(),
        }
    }

    fn update(&mut self, ctx: &egui::Context) {
        // Collect touch/pointer events from egui's input state for this frame.
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Touch { phase, pos, .. } => {
                        match phase {
                            egui::TouchPhase::Start => {
                                self.touch_count += 1;
                                self.touch_pos = Some(*pos);
                                self.last_event =
                                    format!("Touch DOWN ({:.0}, {:.0})", pos.x, pos.y);
                            }
                            egui::TouchPhase::Move => {
                                self.touch_pos = Some(*pos);
                                self.last_event =
                                    format!("Touch MOVE ({:.0}, {:.0})", pos.x, pos.y);
                            }
                            egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                                self.touch_pos = None;
                                self.last_event =
                                    format!("Touch UP ({:.0}, {:.0})", pos.x, pos.y);
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

        let dt = ctx.input(|i| i.unstable_dt);
        let fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };

        // ── Touch ripple indicator ───────────────────────────────────────────
        // Drawn on the foreground layer OUTSIDE the window so it appears on top
        // of everything and always follows the finger position correctly.
        if let Some(pos) = self.touch_pos {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::from("touch_ripple_layer"),
            ));
            painter.circle(
                pos,
                24.0,
                egui::Color32::from_rgba_unmultiplied(255, 200, 0, 60),
                egui::Stroke::new(3.0, egui::Color32::from_rgb(255, 200, 0)),
            );
            // Inner dot
            painter.circle_filled(pos, 5.0, egui::Color32::from_rgb(255, 200, 0));
        }

        // ── Main window ───────────────────────────────────────────────────────
        egui::Window::new(format!("EGUI - FPS: {:.1}", fps))
            .id(egui::Id::from("MainWindow"))
            .default_pos(ctx.viewport_rect().center())
            .default_width(400.0)
            .default_height(300.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.heading("Welcome to EGUI");
                ui.separator();

                ui.group(|ui| {
                    ui.label("Touch Input Status");
                    ui.label(
                        egui::RichText::new(&self.last_event).color(egui::Color32::YELLOW),
                    );
                    ui.label(format!("Total touches: {}", self.touch_count));
                    if let Some(pos) = self.touch_pos {
                        ui.label(format!("Current: ({:.0}, {:.0})", pos.x, pos.y));
                    } else {
                        ui.label(egui::RichText::new("No active touch").weak());
                    }
                });

                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Buttons:");
                    if ui.button("Click Me").clicked() {
                        self.last_event = "Button 'Click Me' tapped!".to_string();
                    }
                    if ui.button("Another Button").clicked() {
                        self.last_event = "Button 'Another' tapped!".to_string();
                    }
                });

                ui.collapsing("Expandable Section", |ui| {
                    ui.label("Additional details.");
                    ui.checkbox(&mut self.checkbox_val, "Sample Checkbox");
                    if self.checkbox_val {
                        ui.label(
                            egui::RichText::new("Checkbox is ON").color(egui::Color32::GREEN),
                        );
                    }
                });
            });
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_level(true)
        .without_time()
        .init();

    // check_su();

    let android_api_level = android::get_api_level()?;
    let android_version = android::get_android_version()?;

    info!("Android Version {android_version} (API {android_api_level})");

    let runtime = AndroidRuntime::load()?;
    let _invocation = runtime.init_invocation()?;

    let vm = runtime.create_java_vm()?;
    let mut env = vm.attach_current_thread()?;

    runtime.start_registration(&mut env)?;

    let bridge = bridge::JavaBridge::new(&mut env)?;
    info!("Bridge initialized");

    bridge.call_main(&mut env)?;

    let (width, height, rotation) = bridge.get_display_size(&mut env)?;

    let (width, height) = if rotation == 0 || rotation == 2 {
        (height, width)
    } else {
        (width, height)
    };

    let window = bridge.create_native_window(&mut env, width, height)?;
    info!("Window Size : {}x{}", window.width(), window.height());

    let mut renderer = renderer::Renderer::new(&window)?;

    // Start the input reader thread.
    // It reads raw Linux multitouch events from /dev/input and converts them to egui events.
    let input_rx = input::start_input_thread(width as f32, height as f32);
    info!("Input thread started");

    let mut app = App::new();

    info!("Starting Render Loop");
    loop {
        // Drain all pending touch events from the input thread before rendering.
        // try_recv is non-blocking so the render loop never stalls waiting for input.
        while let Ok(events) = input_rx.try_recv() {
            // Forward raw egui events into the renderer's next RawInput batch.
            renderer.push_events(events.clone());

            // Also forward touch events to Android's own InputManager via JNI so that
            // other system components can observe them if needed.
            for event in &events {
                if let egui::Event::Touch { id, phase, pos, .. } = event {
                    let action = match phase {
                        egui::TouchPhase::Start => 0,
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => 1,
                        egui::TouchPhase::Move => 2,
                    };
                    // Ignore errors – the JNI inject is best-effort
                    let _ = bridge.inject_touch_event(
                        &mut env,
                        action,
                        id.0 as i64,
                        pos.x as i32,
                        pos.y as i32,
                    );
                }
            }
        }

        renderer.render(|ctx| app.update(ctx));
        renderer.swap_buffers()?;

        if false {
            break;
        }
    }
    todo!("Handle exit gracefully");
}

#[allow(dead_code)]
fn check_su() {
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        #[cfg(debug_assertions)]
        tracing::error!("Try running with: USE_SU=1 cargo run");
        panic!("Error: This application must be run as root (UID 0).")
    }
}
