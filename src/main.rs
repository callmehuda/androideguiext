use anyhow::Result;
use tracing::info;

use crate::android::runtime::AndroidRuntime;

mod android;
mod bridge;
mod dex;
mod jni;
mod renderer;

struct App {
    checkbox_val: bool,
}

impl App {
    fn new() -> Self {
        Self {
            checkbox_val: false,
        }
    }

    fn update(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.unstable_dt);
        let fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        egui::Window::new(format!("EGUI - FPS: {:.1}", fps))
            .id(egui::Id::from("MainWindow"))
            .default_pos(ctx.viewport_rect().center())
            .default_width(400.0)
            .default_height(300.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.heading("Welcome to EGUI");
                ui.separator();

                ui.label("This is a centered window with more content.");
                ui.label(egui::RichText::new("Touch event handling not implemented yet").weak());

                ui.horizontal(|ui| {
                    ui.label("Some buttons:");
                    if ui.button("Click Me").clicked() {
                        // Add interaction logic here
                    }
                    if ui.button("Another Button").clicked() {
                        // Add interaction logic here
                    }
                });

                ui.collapsing("Expandable Section", |ui| {
                    ui.label("Additional details can be shown here.");
                    ui.checkbox(&mut self.checkbox_val, "Sample Checkbox");
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

    let mut app = App::new();

    info!("Starting Render Loop");
    loop {
        renderer.render(|ctx| app.update(ctx));
        renderer.swap_buffers()?;
        // std::thread::sleep(std::time::Duration::from_millis(16)); // ~60 FPS

        //
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
