use std::sync::Arc;
use std::{ffi::c_void, time};

use anyhow::Result;
use glow::HasContext;
use khronos_egl as egl;
use ndk::native_window::NativeWindow;
use tracing::info;

pub struct Renderer {
    egl: Arc<egl::DynamicInstance<egl::EGL1_4>>,
    egl_display: egl::Display,
    egl_surface: egl::Surface,
    #[allow(dead_code)]
    egl_context: egl::Context,
    egui_context: egui::Context,
    egui_painter: egui_glow::Painter,
    egui_raw_input: egui::RawInput,
    pub width: i32,
    pub height: i32,
    start_time: time::Instant,
}

impl Renderer {
    pub fn new(window: &NativeWindow) -> Result<Self> {
        let width = window.width();
        let height = window.height();
        info!("Creating Renderer with size: {}x{}", width, height);

        let egl = unsafe {
            egl::DynamicInstance::<egl::EGL1_4>::load_required_from_filename("libEGL.so")
        }
        .map_err(|e| anyhow::anyhow!("Unable to load libEGL.so: {}", e))?;
        let egl = Arc::new(egl);

        info!("EGL Version: {:?}", egl.version());

        let egl_display = unsafe {
            egl.get_display(egl::DEFAULT_DISPLAY)
                .ok_or(anyhow::anyhow!("Failed to get display"))?
        };

        let (major, minor) = egl.initialize(egl_display)?;
        info!("EGL Initialized: {}.{}", major, minor);

        #[rustfmt::skip]
        let attribs = [
            egl::BLUE_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::RED_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::DEPTH_SIZE, 16,
            egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT,
            egl::SURFACE_TYPE, egl::WINDOW_BIT,
            egl::NONE,
        ];

        let mut configs = vec![];
        let count = egl.matching_config_count(egl_display, &attribs)?;
        configs.reserve(count);
        egl.choose_config(egl_display, &attribs, &mut configs)
            .map_err(|_| anyhow::anyhow!("eglChooseConfig failed"))?;

        let config = *configs
            .first()
            .ok_or(anyhow::anyhow!("No matching EGL config found"))?;

        let format = egl.get_config_attrib(egl_display, config, egl::NATIVE_VISUAL_ID)?;
        window.set_buffers_geometry(0, 0, Some(format.into()))?;

        let context_attribs = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
        let egl_context = egl.create_context(egl_display, config, None, &context_attribs)?;

        let egl_surface = unsafe {
            egl.create_window_surface(egl_display, config, window.ptr().as_ptr() as *mut _, None)?
        };

        egl.make_current(
            egl_display,
            Some(egl_surface),
            Some(egl_surface),
            Some(egl_context),
        )?;

        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map(|f| f as *const c_void)
                    .unwrap_or(std::ptr::null())
            })
        };
        let gl = Arc::new(gl);
        info!("OpenGL Initialized");

        let egui_context = egui::Context::default();

        let egui_painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|e| anyhow::anyhow!("Failed to create painter: {}", e))?;

        let egui_raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            ..Default::default()
        };

        info!("EGUI Context Initialized");

        Ok(Self {
            egl,
            egl_display,
            egl_surface,
            egl_context,
            egui_raw_input,
            egui_context,
            egui_painter,
            width,
            height,
            start_time: time::Instant::now(),
        })
    }

    pub fn render<F: FnOnce(&egui::Context)>(&mut self, run_ui: F) {
        unsafe {
            let gl = self.egui_painter.gl();
            // gl.viewport(0, 0, self.width, self.height);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }

        let ctx = &mut self.egui_context;
        let painter = &mut self.egui_painter;
        self.egui_raw_input.time = Some(self.start_time.elapsed().as_secs_f64());

        ctx.begin_pass(self.egui_raw_input.take());

        run_ui(ctx);

        let full_output = ctx.end_pass();

        // Paint egui primitives
        let clipped_primitives = ctx.tessellate(full_output.shapes, full_output.pixels_per_point);

        painter.paint_and_update_textures(
            [self.width as u32, self.height as u32],
            full_output.pixels_per_point,
            &clipped_primitives,
            &full_output.textures_delta,
        );
    }

    pub fn swap_buffers(&self) -> Result<()> {
        self.egl
            .swap_buffers(self.egl_display, self.egl_surface)
            .map_err(|e| anyhow::anyhow!("Swap buffers failed: {}", e))
    }
}
