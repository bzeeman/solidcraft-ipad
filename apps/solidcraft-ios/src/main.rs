//! SolidCraft on iPadOS: a spike.
//!
//! Runs the same [`solidcraft_ui_egui::SolveApp`] as the desktop and browser apps through eframe's
//! native runner, which on iOS is winit's UIKit backend drawing with wgpu on Metal. It opens the
//! sample part and nothing more: no file dialogs, no settings kept between runs, no touch
//! handling beyond what egui does with a touch (a tap is a click, two fingers zoom).
//!
//! The Apple Pencil's own gestures are tried out here ([`pencil`]): a double tap on its side is
//! Escape, a squeeze clicks where the tip hovered, and a held squeeze opens a ring of the keys a
//! keyboard would give. A readout in the corner says what the Pencil was heard doing.
//!
//! Build for the simulator on Apple silicon with
//! `cargo build -p solidcraft-ios --target aarch64-apple-ios-sim`, then wrap the binary in an app
//! bundle with `packaging/ios/bundle.sh` (the simulator needs no code signature). For a real iPad
//! the target is `aarch64-apple-ios` (`bundle.sh --device`), and the bundle must then be signed.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(any(target_os = "ios", test))]
mod pencil;

#[cfg(target_os = "ios")]
mod ios {
    use solidcraft_engine::Session;
    use solidcraft_ui_egui::ui_kit::{Container, Surface};
    use solidcraft_ui_egui::{Services, SolveApp};

    use crate::pencil;

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

    /// With this in the environment set to `ring`, the app squeezes for itself a moment after it
    /// starts and holds on, so a picture of the simulator (which has no Pencil) shows the ring.
    const DEMO: &str = "SOLIDCRAFT_IOS_PENCIL_DEMO";
    /// The frame the demo's squeeze begins on: the sample is on screen by then.
    const DEMO_FRAME: u32 = 30;

    struct Shell {
        app: SolveApp,
        frames: u32,
        pencil: pencil::Pencil,
        /// See [`DEMO`]: the frames its input has been given for, while it runs.
        demo: Option<u32>,
        /// The Pencil asked to be looked at again after this many seconds (a squeeze that may
        /// become a hold); asked of egui in `logic`.
        wake: Option<f64>,
    }

    impl Shell {
        /// See [`DEMO`]: the tip hovers mid-screen, then a squeeze that is never let go.
        fn demo_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
            let Some(frames) = self.demo else {
                return;
            };
            self.demo = (frames < DEMO_FRAME).then_some(frames + 1);
            let at = ctx.content_rect().center();
            raw.events.push(egui::Event::PointerMoved(at));
            if self.demo.is_none() {
                raw.events.push(egui::Event::Key {
                    key: egui::Key::F14,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }

        /// The ring of keys while it is open, the held modifiers beside the tip, and the readout.
        fn pencil_ui(&mut self, ctx: &egui::Context) {
            let tokens = solidcraft_ui_egui::theme::Tokens::get();
            if let Some(centre) = self.pencil.ring() {
                let bounds = pencil::ring_bounds(centre);
                let held = self.pencil.held();
                let mut pressed = None;
                Container::new(ctx, Surface::Popup).area(egui::Id::unique("pencil-ring")).order(egui::Order::Foreground).fixed_pos(bounds.min).show(
                    ctx,
                    |ui| {
                        // The whole square is the ring's, its dead centre included: a tap between the
                        // keys reaches nothing under it.
                        ui.allocate_rect(bounds, egui::Sense::click());
                        ui.painter().circle(
                            centre,
                            bounds.width() / 2.0,
                            tokens.dialog_body.gamma_multiply(0.9),
                            tokens.primary_border(tokens.dialog_border),
                        );
                        for (index, item) in pencil::ITEMS.iter().enumerate() {
                            let on = item.holds().is_some_and(|modifier| held.contains(modifier));
                            if ui.put(pencil::key_rect(centre, index), egui::Button::new(item.label()).selected(on)).clicked() {
                                pressed = Some(*item);
                            }
                        }
                    },
                );
                if let Some(item) = pressed
                    && let Some(action) = self.pencil.press(item)
                {
                    self.app.synthetic.extend(action.events());
                    ctx.request_repaint();
                }
            }

            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pencil-readout")));
            let font = egui::FontId::proportional(13.0);
            let held = pencil::held_label(self.pencil.held());
            if !held.is_empty()
                && let Some(tip) = self.pencil.tip()
            {
                painter.text(tip + egui::vec2(14.0, -14.0), egui::Align2::LEFT_BOTTOM, held, font.clone(), tokens.accent);
            }
            // Bottom left, over the model and clear of the timeline.
            let content = ctx.content_rect();
            let mut at = egui::pos2(content.left() + 16.0, content.bottom() - 88.0);
            for line in self.pencil.readout().iter().rev() {
                let galley = painter.layout_no_wrap(line.clone(), font.clone(), tokens.text);
                let rect = egui::Rect::from_min_size(egui::pos2(at.x, at.y - galley.size().y), galley.size());
                painter.rect_filled(rect.expand2(egui::vec2(6.0, 2.0)), 4.0, tokens.dialog_body.gamma_multiply(0.8));
                painter.galley(rect.min, galley, tokens.text);
                at.y = rect.top() - 4.0;
            }
        }
    }

    impl eframe::App for Shell {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            if self.frames < 2 {
                self.frames += 1;
                // A second frame right away, which means the first was presented: the app is up.
                ctx.request_repaint();
                log::info!(target: "solidcraft_ios", "frame {} begins", self.frames);
            }
            // What `raw_input_hook` found it needs is asked for here, inside the pass. Asked from
            // the hook, a wait is weighed against the previous pass's request and dropped when
            // that one was "at once": the held squeeze then never opened its ring.
            if let Some(wake) = self.wake.take() {
                ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(wake).unwrap_or_default());
            }
            if self.demo.is_some() || !self.app.synthetic.is_empty() {
                ctx.request_repaint();
            }
            self.app.logic(ctx);
        }

        fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
            self.demo_input(ctx, raw);
            // The Pencil's gestures come out of the input before the app reads it, and what they
            // ask for goes in the way an agent's clicks and keys do.
            let ring = self.pencil.ring().is_some();
            let heard = self.pencil.frame(raw, ctx.content_rect(), !self.app.synthetic.is_empty());
            for action in heard.actions {
                log::info!(target: "solidcraft_ios", "pencil: {action:?}");
                self.app.synthetic.extend(action.events());
            }
            if self.pencil.ring().is_some() != ring {
                log::info!(target: "solidcraft_ios", "pencil: the ring of keys is {}", if ring { "closed" } else { "open" });
            }
            self.wake = heard.wake;
            self.app.raw_input_hook(raw);
        }

        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            // Inside the safe area (the status bar, the home indicator, rounded corners); the
            // strip above it takes the application bar's colour, as on the desktop.
            let content = ui.ctx().content_rect();
            let above = egui::Rect::from_min_max(ui.max_rect().min, egui::pos2(ui.max_rect().right(), content.top()));
            ui.painter().rect_filled(above, 0.0, solidcraft_ui_egui::theme::Tokens::get().app_bar);
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| self.app.ui(ui));
            self.pencil_ui(ui.ctx());
        }
    }

    /// eframe's renderer settings, asking the device for no more than its adapter has. The limits
    /// asked for by default are WebGPU's, and a Metal device older than the Apple4 GPU family
    /// (before the A11) has 15 inter-stage variables where WebGPU's default is 16. The Simulator
    /// always reports the Apple2 family, whatever iPad it shows, so there the default request
    /// fails and the window never opens.
    fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
        let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
            let base = setup.device_descriptor.clone();
            setup.device_descriptor = std::sync::Arc::new(move |adapter| {
                let mut descriptor = base(adapter);
                descriptor.required_limits = descriptor.required_limits.or_worse_values_from(&adapter.limits());
                descriptor
            });
        }
        options
    }

    pub fn run() {
        if log::set_logger(&Stderr).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
        let options = eframe::NativeOptions {
            wgpu_options: wgpu_options(),
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
                        // What the Simulator's device lacks next to a real iPad's.
                        let downlevel = rs.adapter.get_downlevel_capabilities();
                        if !downlevel.is_webgpu_compliant() {
                            let missing = eframe::wgpu::DownlevelFlags::compliant() - downlevel.flags;
                            log::info!(target: "solidcraft_ios", "graphics: the device is below WebGPU's baseline, without {missing:?}");
                        }
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
                let demo = std::env::var(DEMO).is_ok_and(|demo| demo == "ring").then_some(0);
                Ok(Box::new(Shell { app, frames: 0, pencil: pencil::Pencil::default(), demo, wake: None }))
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
