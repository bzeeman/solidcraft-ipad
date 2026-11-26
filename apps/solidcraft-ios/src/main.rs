//! SolidCraft on iPadOS: a spike.
//!
//! Runs the same [`solidcraft_ui_egui::SolveApp`] as the desktop and browser apps through eframe's
//! native runner, which on iOS is winit's UIKit backend drawing with wgpu on Metal. It opens the
//! sample part first. Files come and go through the system's document picker ([`documents`]):
//! Open, Import, Save As and Export reach iCloud Drive, On My iPad and other apps' folders, and the
//! app's own Documents folder shows in the Files app. The settings are kept between runs.
//!
//! Fingers move the view and the Apple Pencil points ([`touch`]): one finger turns the model, two
//! carry it and pinch to zoom, a finger's tap is a click, a finger held still is a right click,
//! and the Pencil's tip is the mouse's left button. The Pencil's own gestures are tried out here
//! too ([`pencil`]): a double tap on its side is Escape, a squeeze clicks where the tip hovered,
//! and a held squeeze opens a ring of the keys a keyboard would give (its Right click makes the
//! next click a right click). A readout in the corner says what fingers and Pencil were heard doing.
//!
//! Text boxes get keys of our own instead of the system's keyboard ([`keys`]): a number pad beside
//! an expression box, a compact keyboard for any other, each dragged where one wants it by its
//! handle; Full on either brings up the system's whole keyboard.
//!
//! Build for the simulator on Apple silicon with
//! `cargo build -p solidcraft-ios --target aarch64-apple-ios-sim`, then wrap the binary in an app
//! bundle with `packaging/ios/bundle.sh` (the simulator needs no code signature). For a real iPad
//! the target is `aarch64-apple-ios` (`bundle.sh --device`), and the bundle must then be signed.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "ios")]
mod agent;
#[cfg(any(target_os = "ios", test))]
mod bookmarks;
#[cfg(target_os = "ios")]
mod documents;
#[cfg(any(target_os = "ios", test))]
mod input;
#[cfg(any(target_os = "ios", test))]
mod keys;
#[cfg(any(target_os = "ios", test))]
mod pencil;
#[cfg(any(target_os = "ios", test))]
mod touch;

#[cfg(target_os = "ios")]
mod ios {
    use solidcraft_engine::Session;
    use solidcraft_ui_egui::ui_kit::{Container, Surface};
    use solidcraft_ui_egui::{Services, SolveApp};

    use crate::{agent, documents, input, keys, pencil};

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

    /// How far the ring's disc reaches past its keys.
    const RING_RIM: f32 = 6.0;
    /// The radius of the ring marking a held finger, in points: wider than a fingertip.
    const FINGER_MARK: f32 = 34.0;

    struct Shell {
        /// Boxed: the app is about 190 KB, so it lives on the heap, not the main thread's stack.
        app: Box<SolveApp>,
        frames: u32,
        /// The Pencil, the fingers and what they do to a frame's input before the app reads it.
        input: input::Input,
        /// Our number pad and compact keyboard, which stand in for the system's keyboard.
        keys: keys::Keys,
    }

    impl Shell {
        /// Our keys for the text box that has the focus: the number pad for an expression box, the
        /// compact keyboard for any other, or the key beside a box that has the system's keyboard
        /// instead. Runs after the app has drawn, so the box and where it is are this frame's; while
        /// our keys show, the system's keyboard stays down.
        fn text_keys(&mut self, ctx: &egui::Context) {
            let tokens = solidcraft_theme::Tokens::get();
            let rect = ctx.output(|o| o.ime.as_ref().map(|ime| ime.rect));
            let expression = solidcraft_ui_egui::params_dialog::focused_expression(ctx);
            let field = ctx.memory(|m| m.focused()).zip(rect).map(|(id, rect)| keys::Field { id, rect, expression: expression == Some(id) });
            let names = if field.is_some_and(|f| f.expression) { solidcraft_ui_egui::params_dialog::parameter_names(ctx) } else { Vec::new() };
            let lit = self.keys.lit(ctx.input(|i| i.time));
            let (shown, system) = self.keys.frame(field, names, ctx.content_rect());
            if !system {
                // egui-winit raises the system's keyboard while a frame asks for text input.
                ctx.output_mut(|o| o.ime = None);
            }
            let Some(shown) = shown else { return };
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("text-keys")));
            if shown.ours() {
                painter.rect(shown.outline, 10.0, tokens.dialog_body, tokens.primary_border(tokens.dialog_border), egui::StrokeKind::Outside);
            }
            for (rect, key) in &shown.keys {
                if *key == keys::Key::Handle {
                    // A short bar in the middle of the handle's strip says it can be taken hold of.
                    let bar = egui::Rect::from_center_size(rect.center(), egui::vec2(44.0, 5.0));
                    painter.rect_filled(bar, 2.5, tokens.text_dim);
                    continue;
                }
                let label = match key {
                    keys::Key::Name(index) => shown.names.get(*index).cloned().unwrap_or_default(),
                    other => other.label(shown.shift),
                };
                let on = lit.is_some_and(|(k, _)| k == *key) || (*key == keys::Key::Shift && shown.shift);
                let (fill, ink) = if on { (tokens.accent, tokens.on_accent) } else { (tokens.control_fill, tokens.text) };
                painter.rect(*rect, 8.0, fill, tokens.control_border_stroke(tokens.control_border), egui::StrokeKind::Inside);
                let size = if key.types() { 21.0 } else { 15.0 };
                let galley = painter.layout(label, egui::FontId::proportional(size), ink, rect.width() - 8.0);
                let at = rect.center() - galley.size() / 2.0;
                painter.with_clip_rect(rect.shrink(2.0)).galley(at, galley, ink);
            }
            if let Some((_, left)) = lit {
                ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(left).unwrap_or_default());
            }
        }

        /// The ring of keys while it is open, the held modifiers beside the tip, and the readout.
        fn pencil_ui(&mut self, ctx: &egui::Context) {
            let tokens = solidcraft_theme::Tokens::get();
            if let Some(centre) = self.input.pencil.ring() {
                let bounds = pencil::ring_bounds(centre);
                let pencil = &self.input.pencil;
                let mut pressed = None;
                Container::new(ctx, Surface::Popup).area(egui::Id::unique("pencil-ring")).order(egui::Order::Foreground).fixed_pos(bounds.min).show(
                    ctx,
                    |ui| {
                        // The whole square is the ring's, its dead centre included: a tap between the
                        // keys reaches nothing under it.
                        ui.allocate_rect(bounds, egui::Sense::click());
                        // A rim of disc shows around the keys, which are round and so stay inside it.
                        ui.painter().circle(
                            centre,
                            bounds.width() / 2.0 + RING_RIM,
                            tokens.dialog_body.gamma_multiply(0.9),
                            tokens.primary_border(tokens.dialog_border),
                        );
                        for (index, item) in pencil::ITEMS.iter().enumerate() {
                            let on = pencil.is_on(*item);
                            // A dialog's controls on a dialog's body: the pair the theme draws
                            // apart. A key that is on takes the selection's colours instead.
                            let key = egui::Button::new(item.label()).corner_radius(pencil::KEY_SIDE / 2.0);
                            let key = if on {
                                key.selected(true)
                            } else {
                                key.fill(tokens.control_fill).stroke(tokens.control_border_stroke(tokens.control_border))
                            };
                            if ui.put(pencil::key_rect(centre, index), key).clicked() {
                                pressed = Some(*item);
                            }
                        }
                    },
                );
                if let Some(item) = pressed
                    && let Some(action) = self.input.pencil.press(item)
                {
                    self.app.synthetic.extend(action.events());
                    ctx.request_repaint();
                }
            }

            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pencil-readout")));
            let font = egui::FontId::proportional(13.0);
            if let Some(at) = self.input.held {
                // A held finger: a ring around it, outside the fingertip, and Shift beside it.
                painter.circle_stroke(at, FINGER_MARK, egui::Stroke::new(3.0, tokens.accent));
                painter.text(at + egui::vec2(0.0, -FINGER_MARK - 6.0), egui::Align2::CENTER_BOTTOM, "Shift", font.clone(), tokens.accent);
            }
            let held = self.input.pencil.held_text();
            if !held.is_empty()
                && let Some(tip) = self.input.pencil.tip()
            {
                painter.text(tip + egui::vec2(14.0, -14.0), egui::Align2::LEFT_BOTTOM, held, font.clone(), tokens.accent);
            }
            // Bottom left, over the model and clear of the timeline.
            let content = ctx.content_rect();
            let mut at = egui::pos2(content.left() + 16.0, content.bottom() - 88.0);
            for line in self.input.readout().iter().rev() {
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
            // What the input hook found it needs is asked for here, inside the pass (see
            // `Input::wake`).
            if let Some(wake) = self.input.wake.take() {
                ctx.request_repaint_after(std::time::Duration::try_from_secs_f64(wake).unwrap_or_default());
            }
            if self.input.busy() || !self.app.synthetic.is_empty() {
                ctx.request_repaint();
            }
            // Recent designs brought back from their bookmarks, and designs tapped in Files; not
            // while a picker is answering.
            let waiting = if self.app.file_picker_busy() { Vec::new() } else { documents::take().into_iter().chain(documents::take_opened()).collect() };
            for path in waiting {
                self.app.open_path_async(&path);
            }
            self.app.logic(ctx);
        }

        fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
            self.input.frame(&mut self.app, ctx.content_rect(), raw);
            if self.keys.input(raw) {
                ctx.request_repaint();
            }
            self.app.raw_input_hook(raw);
        }

        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            // Inside the safe area (the status bar, the home indicator, rounded corners); the
            // strip above it takes the application bar's colour, as on the desktop.
            let content = ui.ctx().content_rect();
            let above = egui::Rect::from_min_max(ui.max_rect().min, egui::pos2(ui.max_rect().right(), content.top()));
            ui.painter().rect_filled(above, 0.0, solidcraft_theme::Tokens::get().app_bar);
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| self.app.ui(ui));
            self.text_keys(ui.ctx());
            self.pencil_ui(ui.ctx());
        }

        /// eframe calls this every 30 seconds and when the app closes. iOS may end an app in the
        /// background without closing it, so a change can be lost if it is younger than that.
        fn save(&mut self, storage: &mut dyn eframe::Storage) {
            match self.app.try_prefs() {
                Ok(prefs) => storage.set_string(solidcraft_ui_egui::prefs::STORAGE_KEY, prefs),
                Err(reason) => log::warn!(target: "solidcraft_ios", "settings were not written: {reason}"),
            }
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
            // The window is the whole screen, so only the settings are kept between runs.
            persist_window: false,
            // eframe keeps no settings on iOS by itself (its `storage_dir` has no iOS folder).
            persistence_path: solidcraft_engine::recovery::data_dir().map(|dir| dir.join("app.ron")),
            ..Default::default()
        };
        let result = eframe::run_native(
            "SolidCraft",
            options,
            Box::new(|cc| {
                log::info!(target: "solidcraft_ios", "window and device ready");
                solidcraft_ui_egui::fonts::setup_bundled_fonts(&cc.egui_ctx);
                let mut services = Services::default();
                documents::install(&mut services, &cc.egui_ctx);
                // The agent chat's endpoint agents: their model requests, and SolidCraft's tools in
                // this process (iPadOS starts no helper).
                services.agent_http = Some(std::sync::Arc::new(agent::http));
                let mut app = Box::new(SolveApp::new(Session::default(), services));
                app.agent_chat.in_process_tools = Some(agent::starter());
                // The settings of the last run, before the iPad's own choices below override any.
                if let Some(stored) = cc.storage.and_then(|s| solidcraft_ui_egui::prefs::stored(|k| s.get_string(k))) {
                    app.load_prefs(&stored);
                }
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
                // A finger cannot be put back at the other edge of the view, as a mouse's pointer
                // is when an orbit carries it there.
                app.preferences.orbit_cursor_wrap = false;
                app.session.doc_mut().name = "Sample Plate".into();
                app.session.mark_saved();
                app.session.undo.clear();
                // Built straight into the session, so nothing asked for a fit: frame it at the
                // home view as any opened design is. The fit waits for the first layout.
                app.cam.set_view(solidcraft_viewport::StandardView::Iso);
                app.fit_view();
                log::info!(target: "solidcraft_ios", "app created");
                Ok(Box::new(Shell { app, frames: 0, input: input::Input::from_env(), keys: keys::Keys::default() }))
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
