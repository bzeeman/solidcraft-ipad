//! SolidCraft on iPadOS: a spike.
//!
//! Runs the same [`solidcraft_ui_egui::SolveApp`] as the desktop and browser apps through eframe's
//! native runner, which on iOS is winit's UIKit backend drawing with wgpu on Metal. It opens the
//! sample part and nothing more: no file dialogs, no settings kept between runs, no touch or
//! Apple Pencil handling beyond what egui does with a touch (a tap is a click, two fingers zoom).
//!
//! Build for the simulator on Apple silicon with
//! `cargo build -p solidcraft-ios --target aarch64-apple-ios-sim`, then wrap the binary in an app
//! bundle with `packaging/ios/bundle-sim.sh` (the simulator needs no code signature).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "ios")]
mod ios {
    use solidcraft_engine::Session;
    use solidcraft_ui_egui::{Services, SolveApp};

    /// The scene's depth and stencil, as in the desktop app.
    const DEPTH_BITS: u8 = 24;
    const STENCIL_BITS: u8 = 8;
    /// Anti-aliasing the scene asks for; the device's answer may be lower.
    const MSAA: u32 = 4;

    /// Everything the app logs goes to stderr, which the simulator hands to whoever launched the
    /// app (`xcrun simctl launch --console`, or its `--stderr` file).
    struct Stderr;

    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= if m.target().starts_with("solidcraft") { log::LevelFilter::Info } else { log::LevelFilter::Warn }
        }

        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                use std::io::Write as _;
                // A closed stderr is no reason to stop.
                let _ = writeln!(std::io::stderr().lock(), "[{} {}] {}", r.level().as_str().to_ascii_lowercase(), r.target(), r.args());
            }
        }

        fn flush(&self) {}
    }

    struct Shell {
        app: SolveApp,
        frames: u32,
    }

    impl eframe::App for Shell {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            if self.frames < 2 {
                self.frames += 1;
                // A second frame right away, which means the first was presented: the app is up.
                ctx.request_repaint();
                log::info!(target: "solidcraft_ios", "frame {} begins", self.frames);
            }
            self.app.logic(ctx);
        }

        fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
            self.app.raw_input_hook(raw);
        }

        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            // Inside the safe area (the status bar, the home indicator, rounded corners); the
            // strip above it takes the application bar's colour, as on the desktop.
            let content = ui.ctx().content_rect();
            let above = egui::Rect::from_min_max(ui.max_rect().min, egui::pos2(ui.max_rect().right(), content.top()));
            ui.painter().rect_filled(above, 0.0, solidcraft_ui_egui::theme::Tokens::get().app_bar);
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| self.app.ui(ui));
        }
    }

    pub fn run() {
        if log::set_logger(&Stderr).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
        let options = eframe::NativeOptions {
            // The window's pass (egui's) has no depth and one sample: the viewport draws its scene
            // offscreen with its own depth and MSAA, then blits it (as the desktop app does).
            depth_buffer: 0,
            stencil_buffer: 0,
            multisampling: 1,
            // Nothing is kept between runs yet.
            persist_window: false,
            ..Default::default()
        };
        let result = eframe::run_native(
            "SolidCraft",
            options,
            Box::new(|cc| {
                log::info!(target: "solidcraft_ios", "window and device ready");
                solidcraft_ui_egui::fonts::setup_bundled_fonts(&cc.egui_ctx);
                let mut app = SolveApp::new(Session::default(), Services::default());
                match &cc.wgpu_render_state {
                    Some(rs) => {
                        let info = rs.adapter.get_info();
                        let depth = eframe::egui_wgpu::depth_format_from_bits(DEPTH_BITS, STENCIL_BITS);
                        let supported = eframe::egui_wgpu::supported_msaa_samples(&rs.adapter, &rs.device, rs.target_format, depth);
                        let samples = supported.iter().copied().filter(|&n| n <= MSAA).max().unwrap_or(1);
                        let samples = app.set_wgpu_split(rs, DEPTH_BITS, STENCIL_BITS, samples);
                        app.msaa_supported = supported;
                        app.preferences.msaa = samples;
                        solidcraft_ui_egui::gpu::install_picture_renderer(rs);
                        log::info!(target: "solidcraft_ios", "graphics: {:?} {} ({}), MSAA {samples}x", info.backend, info.name, info.driver);
                    }
                    None => log::error!(target: "solidcraft_ios", "no wgpu render state: the viewport stays empty"),
                }
                if let Err(e) = app.session.run_script(&solidcraft_engine::sample::script()) {
                    app.set_status(format!("sample: {e}"), true);
                    log::error!(target: "solidcraft_ios", "sample: {e}");
                }
                app.session.doc_mut().name = "Sample Plate".into();
                app.session.mark_saved();
                app.session.undo.clear();
                // Built straight into the session, so nothing asked for a fit: frame it at the
                // home view as any opened design is. The fit waits for the first layout.
                app.cam.set_view(solidcraft_engine::render::StandardView::Iso);
                app.fit_view();
                log::info!(target: "solidcraft_ios", "app created");
                Ok(Box::new(Shell { app, frames: 0 }))
            }),
        );
        if let Err(e) = result {
            log::error!(target: "solidcraft_ios", "the window could not open: {e}");
        }
    }
}

#[cfg(target_os = "ios")]
fn main() {
    ios::run();
}

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("solidcraft-ios only runs on iOS and iPadOS: build it with `cargo build -p solidcraft-ios --target aarch64-apple-ios-sim`");
}
