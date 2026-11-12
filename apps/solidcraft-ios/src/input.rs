//! What the iPad spike does with a frame's input before the app reads it.
//!
//! The touches come first ([`crate::touch`]): they decide whose the pointer is, the Pencil's or a
//! finger's, and what fingers do with the view. Then the Pencil's own gestures
//! ([`crate::pencil`]). [`Input::frame`] runs both, and the two self-tests a simulator on a build
//! machine needs because it has neither a Pencil nor fingers.
//!
//! Nothing here is specific to iOS: it drives the shared app through its public fields, so the
//! fingers' self-test also runs headless as a test on any platform.

use egui::{Pos2, Rect};
use solidcraft_engine::render::Camera;
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::viewport::NavMode;

use crate::{pencil, touch};

/// With this in the environment set to `ring`, the app squeezes for itself a moment after it
/// starts and holds on, so a picture of the simulator (which has no Pencil) shows the ring.
const DEMO: &str = "SOLIDCRAFT_IOS_PENCIL_DEMO";
/// The frame the demo's squeeze begins on, and the first the fingers' self-test may begin on: the
/// sample is on screen by then.
const DEMO_FRAME: u32 = 30;
/// With this in the environment set to `camera`, the app puts fingers of its own on the model
/// once its view stands still ([`touch::demo`]): one taps, two carry the model, two spread, one
/// drags. After each it says what the selection and the camera did and whether that is what
/// fingers should do, and at the end one line that says whether all of it was. The simulator on
/// a build machine has no fingers.
const TOUCH_DEMO: &str = "SOLIDCRAFT_IOS_TOUCH_DEMO";
/// The self-test's fingers wait until the camera has stood still for this many frames: the first
/// frames fit the model to the view.
const SETTLED: u32 = 10;
/// The self-test's fingers take a step no sooner than this (seconds) after the last, whatever the
/// frame rate: a finger begins no drag in its first moments on the screen ([`touch::SETTLE`]),
/// and at a few hundred frames a second its whole drag would fit into them.
const DEMO_STEP: f64 = 0.015;

/// Which self-tests run for these values of [`DEMO`] and [`TOUCH_DEMO`]: the Pencil's, the
/// fingers'. Never both: the Pencil's leaves its ring of keys open over the middle of the view,
/// where the fingers' test pinches and drags. A simulator hands the app of a second launch the
/// variables of the first as well (run 43136793893: the ring opened in the fingers' run, whose
/// launch asked for no ring, and their pinch and drag landed on it), so both can be set where
/// one was asked for; the fingers' test then runs, being the one that judges itself.
fn asked(pencil: Option<&str>, fingers: Option<&str>) -> (bool, bool) {
    let camera = fingers == Some("camera");
    (pencil == Some("ring") && !camera, camera)
}

/// Where the camera looks: what the self-test compares before and after its fingers.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    target: [f64; 3],
    /// From the target toward the eye, of length 1.
    back: [f64; 3],
    /// Half of what the view shows from top to bottom, in the model's units.
    half_height: f64,
}

impl Look {
    fn of(app: &SolveApp) -> Self {
        let (target, back) = (app.cam.target, app.cam.back());
        Self { target: [target.x, target.y, target.z], back: [back.x, back.y, back.z], half_height: app.cam.half_height() }
    }

    /// How far the target is from `from`'s, in the model's units.
    fn carried(self, from: Self) -> f64 {
        self.target.iter().zip(from.target).map(|(to, from)| (to - from).powi(2)).sum::<f64>().sqrt()
    }

    /// The angle between this view's direction and `from`'s, in radians.
    fn turned(self, from: Self) -> f64 {
        self.back.iter().zip(from.back).map(|(to, from)| to * from).sum::<f64>().clamp(-1.0, 1.0).acos()
    }

    /// How many times larger the model is drawn than in `from`.
    fn zoomed(self, from: Self) -> f64 {
        from.half_height / self.half_height
    }
}

/// See [`TOUCH_DEMO`].
#[derive(Debug, Default)]
struct TouchDemo {
    /// The frames waited for the view to stand still, and how many in a row it has.
    frames: u32,
    still: u32,
    last: Option<Look>,
    /// The next step of the fingers, once they have begun, and when the last one was taken.
    step: Option<u16>,
    stepped: Option<f64>,
    /// The camera when the part now running began.
    from: Option<Look>,
    /// The parts that did not do what fingers should.
    failed: Vec<&'static str>,
}

/// The Pencil, the fingers and the self-tests, from one frame to the next.
#[derive(Debug, Default)]
pub struct Input {
    pub pencil: pencil::Pencil,
    touches: touch::Touches,
    /// A finger's drag has lit the navigation bar's Orbit mode; it goes out with the drag.
    orbit_lit: bool,
    /// The camera as that drag found it: where the view goes back to if the drag turns out to
    /// be the first of two fingers ([`touch::Heard::taken_back`]).
    before: Option<Camera>,
    /// See [`DEMO`]: the frames its input has been given for, while it runs.
    demo: Option<u32>,
    /// See [`TOUCH_DEMO`], while it runs.
    touch_demo: Option<TouchDemo>,
    /// What the fingers' self-test has said so far, a line each.
    touch_report: Vec<String>,
    /// The Pencil or a finger asked to be looked at again after this many seconds (a squeeze
    /// that may become a hold, a tap's release). The shell asks egui inside its pass: asked
    /// from the input hook, a wait is weighed against the previous pass's request and dropped
    /// when that one was "at once", and the held squeeze then never opened its ring.
    pub wake: Option<f64>,
}

impl Input {
    /// With the self-test the environment asks for ([`DEMO`], [`TOUCH_DEMO`]), and a line in
    /// the log that says what it asked.
    pub fn from_env() -> Self {
        let (pencil, fingers) = (std::env::var(DEMO).ok(), std::env::var(TOUCH_DEMO).ok());
        let (ring, camera) = asked(pencil.as_deref(), fingers.as_deref());
        if pencil.is_some() || fingers.is_some() {
            let run = match (ring, camera) {
                (_, true) => "the fingers' self-test runs",
                (true, false) => "the Pencil's self-test runs",
                (false, false) => "no self-test runs",
            };
            log::info!(target: "solidcraft_ios", "self-tests: {DEMO}={pencil:?}, {TOUCH_DEMO}={fingers:?}: {run}");
        }
        Self { demo: ring.then_some(0), touch_demo: camera.then(TouchDemo::default), ..Self::default() }
    }

    /// A self-test runs: frames must keep coming though no input does.
    pub fn busy(&self) -> bool {
        self.demo.is_some() || self.touch_demo.is_some()
    }

    /// Read one frame's input before the app does. `content` is the screen inside the safe area.
    /// What the Pencil's gestures ask for goes into the app the way an agent's clicks and keys
    /// do.
    pub fn frame(&mut self, app: &mut SolveApp, content: Rect, raw: &mut egui::RawInput) {
        self.touch_demo_input(app, raw);
        let touched = self.touches.frame(raw);
        for note in &touched.notes {
            log::info!(target: "solidcraft_ios", "touch: {note}");
        }
        self.finger_drag(app, touched.drag, touched.taken_back);
        self.pencil.fingers(touched.fingers);
        self.demo_input(content, raw);
        let ring = self.pencil.ring().is_some();
        let heard = self.pencil.frame(raw, content, !app.synthetic.is_empty());
        for action in heard.actions {
            log::info!(target: "solidcraft_ios", "pencil: {action:?}");
            app.synthetic.extend(action.events());
        }
        if self.pencil.ring().is_some() != ring {
            log::info!(target: "solidcraft_ios", "pencil: the ring of keys is {}", if ring { "closed" } else { "open" });
        }
        self.wake = match (heard.wake, touched.wake) {
            (Some(pencil), Some(fingers)) => Some(pencil.min(fingers)),
            (pencil, fingers) => pencil.or(fingers),
        };
    }

    /// What the Pencil and the fingers are doing and their latest gestures, a line each, for the
    /// spike's readout; and what the fingers' self-test said, when it ran.
    pub fn readout(&self) -> Vec<String> {
        self.pencil.readout().into_iter().chain(self.touches.readout()).chain(self.touch_report.iter().cloned()).collect()
    }

    /// One finger dragging turns the model, as the spec for touch has it. The camera already
    /// turns with a left-button drag while the navigation bar's Orbit mode is lit, and leaves a
    /// still press, the panels and the Pencil alone: so a finger's drag that began over the view
    /// lights that mode, and it goes out the frame after the finger lifts (the release's frame is
    /// still the drag's, or its end would select). A mode the user lit stays theirs.
    ///
    /// A drag `taken_back` was no drag: the first of two fingers that came down one after the
    /// other, already on its way. The view goes back to where that drag found it, before the
    /// app reads the frame in which the two fingers begin.
    fn finger_drag(&mut self, app: &mut SolveApp, drag: Option<Pos2>, taken_back: bool) {
        if taken_back && let Some(before) = self.before.take() {
            app.cam = before;
            // The turn the camera code keeps for itself between a drag's frames.
            app.viewport.orbit_free = None;
        }
        match (drag, self.orbit_lit) {
            (Some(origin), false) => {
                let over_view = app.viewport.rect.is_none_or(|view| view.contains(origin));
                if over_view && app.viewport.nav.is_none() {
                    app.viewport.nav = Some(NavMode::Orbit);
                    self.orbit_lit = true;
                    self.before = Some(app.cam);
                }
            }
            (None, true) => {
                if app.viewport.nav == Some(NavMode::Orbit) {
                    app.viewport.nav = None;
                }
                self.orbit_lit = false;
                self.before = None;
            }
            _ => {}
        }
    }

    /// See [`DEMO`]: the tip hovers mid-screen, then a squeeze that is never let go.
    fn demo_input(&mut self, content: Rect, raw: &mut egui::RawInput) {
        let Some(frames) = self.demo else {
            return;
        };
        self.demo = (frames < DEMO_FRAME).then_some(frames + 1);
        raw.events.push(egui::Event::PointerMoved(content.center()));
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

    fn report(&mut self, line: String) {
        log::info!(target: "solidcraft_ios", "{line}");
        self.touch_report.push(line);
    }

    /// See [`TOUCH_DEMO`]: this frame's touches of the self-test's fingers, and what the part
    /// that has just ended did.
    fn touch_demo_input(&mut self, app: &SolveApp, raw: &mut egui::RawInput) {
        let Some(demo) = &mut self.touch_demo else {
            return;
        };
        let Some(view) = app.viewport.rect else {
            return;
        };
        let now = raw.time.filter(|time| time.is_finite());
        if let (Some(now), Some(stepped)) = (now, demo.stepped)
            && (0.0..DEMO_STEP).contains(&(now - stepped))
        {
            return;
        }
        let look = Look::of(app);
        let mut lines = Vec::new();
        let step = match demo.step {
            Some(step) => step,
            None => {
                demo.frames += 1;
                demo.still = if demo.last == Some(look) { demo.still + 1 } else { 0 };
                demo.last = Some(look);
                if demo.frames < DEMO_FRAME || demo.still < SETTLED {
                    return;
                }
                lines.push(format!(
                    "touch demo: fingers come down on a view of {:.0} x {:.0} pt, after {} frames",
                    view.width(),
                    view.height(),
                    demo.frames
                ));
                demo.from = Some(look);
                0
            }
        };
        let from = demo.from.unwrap_or(look);
        let (carried, turned, zoomed) = (look.carried(from), look.turned(from), look.zoomed(from));
        let selected = app.session.selection.len();
        // The camera carries the model this far for a point of the fingers' way.
        let per_point = 2.0 * from.half_height / f64::from(view.height().max(1.0));
        let ended = match step {
            touch::DEMO_TAP_DONE => Some(("tap", selected > 0 && look == from, "it should select what is under it and leave the camera".to_owned())),
            touch::DEMO_PAN_DONE => {
                let expected = f64::from(touch::DEMO_PAN.length()) * per_point;
                let ok = (carried - expected).abs() <= expected * 0.1 && turned < 1e-6 && (zoomed - 1.0).abs() < 1e-6;
                Some(("two fingers carrying", ok, format!("the model should be carried {expected:.3} and nothing else")))
            }
            touch::DEMO_PINCH_DONE => {
                let expected = f64::from(touch::DEMO_PINCH);
                Some((
                    "two fingers spreading",
                    (zoomed - expected).abs() <= expected * 0.05 && turned < 1e-6,
                    format!("the zoom should be x{expected:.2}, with no turn"),
                ))
            }
            touch::DEMO_DRAG_DONE => {
                Some(("one finger dragging", turned > 0.2 && (zoomed - 1.0).abs() < 0.01, "the view should turn, with no zoom".to_owned()))
            }
            _ => None,
        };
        if let Some((part, ok, expected)) = ended {
            let verdict = if ok { "as expected" } else { "NOT as expected" };
            lines.push(format!(
                "touch demo: {part}: carried {carried:.3}, turned {turned:.3} rad, zoom x{zoomed:.3}, {selected} selected: {verdict} ({expected})"
            ));
            if !ok {
                demo.failed.push(part);
            }
            demo.from = Some(look);
        }
        match touch::demo(step, view) {
            Some(events) => {
                raw.events.extend(events);
                demo.step = Some(step + 1);
                demo.stepped = now;
            }
            None => {
                lines.push(if demo.failed.is_empty() {
                    "touch demo: the camera moved as expected".to_owned()
                } else {
                    format!("touch demo: FAILED: {}", demo.failed.join(", "))
                });
                self.touch_demo = None;
            }
        }
        for line in lines {
            self.report(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, TouchDeviceId, TouchId, TouchPhase, vec2};
    use solidcraft_ui_egui::scenario::Harness;

    use super::*;

    /// The app, headless, with the sample part at the home view as the iPad app opens it.
    fn sample() -> Harness {
        let mut h = Harness::new();
        h.app.session.run_script(&solidcraft_engine::sample::script()).unwrap();
        h.app.cam.set_view(solidcraft_engine::render::StandardView::Iso);
        h.app.fit_view();
        h.app.preferences.orbit_cursor_wrap = false;
        h.frames(5);
        h
    }

    /// The headless app's window.
    fn content() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))
    }

    /// One frame of the app with `events` from the window layer, read by `input` first.
    fn frame(h: &mut Harness, input: &mut Input, time: &mut f64, events: Vec<Event>) {
        *time += 1.0 / 60.0;
        let mut raw = egui::RawInput { time: Some(*time), events, ..Default::default() };
        input.frame(&mut h.app, content(), &mut raw);
        h.frame_with(raw.events);
    }

    #[test]
    fn the_apps_own_fingers_select_carry_zoom_and_turn_the_sample() {
        let mut h = sample();
        let mut input = Input { touch_demo: Some(TouchDemo::default()), ..Default::default() };
        let mut time = 10.0;
        for _ in 0..600 {
            if !input.busy() {
                break;
            }
            frame(&mut h, &mut input, &mut time, Vec::new());
        }
        assert!(!input.busy(), "the self-test never ended: {:#?}", input.touch_report);
        assert_eq!(input.touch_report.last().map(String::as_str), Some("touch demo: the camera moved as expected"), "{:#?}", input.touch_report);
        assert_eq!(h.app.viewport.nav, None, "the Orbit mode the drag lit is out again");
        assert_eq!(input.readout().last().map(String::as_str), Some("touch demo: the camera moved as expected"));
    }

    #[test]
    fn a_tap_of_the_pencils_tip_selects_what_is_under_it() {
        let mut h = sample();
        let mut input = Input::default();
        let mut time = 10.0;
        let at = h.app.viewport.rect.unwrap().center();
        let tip = |phase: TouchPhase| Event::Touch { device_id: TouchDeviceId(0), id: TouchId(9), phase, pos: at, force: Some(0.4) };
        assert!(h.app.session.selection.is_empty());
        // As the window layer reports it: the touch, then the pointer it makes of it, which
        // leaves the screen in the frame of the release.
        frame(&mut h, &mut input, &mut time, vec![tip(TouchPhase::Start), Event::PointerMoved(at), pencil_button(at, true)]);
        frame(&mut h, &mut input, &mut time, Vec::new());
        frame(&mut h, &mut input, &mut time, vec![tip(TouchPhase::End), pencil_button(at, false), Event::PointerGone]);
        assert_eq!(input.wake, Some(0.0), "a frame is owed in which the pointer goes");
        for _ in 0..3 {
            frame(&mut h, &mut input, &mut time, Vec::new());
        }
        assert_eq!(h.app.session.selection.len(), 1, "{:?}", h.app.session.selection);
        assert!(h.ctx.input(|i| i.pointer.latest_pos().is_none()), "the pointer has left the screen with the tip");
    }

    /// A finger as the window layer reports it.
    fn finger(id: u64, phase: TouchPhase, at: Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(0), id: TouchId(id), phase, pos: at, force: None }
    }

    /// The fault the iPad's log showed in 3 of Ben's first 10 two-finger gestures: the first
    /// finger down a moment before the second, and already moving. The view turned before it
    /// was carried.
    #[test]
    fn two_fingers_that_land_one_after_the_other_carry_the_model_and_do_not_turn_it() {
        // The frames between the two fingers: 0.05 s, inside the first one's first moments; and
        // 0.15 s, by when the first has begun to turn the view.
        for lead in [3_u16, 9] {
            let mut h = sample();
            let mut input = Input::default();
            let mut time = 10.0;
            let view = h.app.viewport.rect.unwrap();
            let from = Look::of(&h.app);
            let step = vec2(6.0, 3.0);
            let (a, b) = (view.center() - vec2(60.0, 0.0), view.center() + vec2(60.0, 0.0));
            frame(&mut h, &mut input, &mut time, vec![finger(1, TouchPhase::Start, a)]);
            let mut turned = 0.0_f64;
            for n in 1..lead {
                frame(&mut h, &mut input, &mut time, vec![finger(1, TouchPhase::Move, a + step * f32::from(n))]);
                turned = turned.max(Look::of(&h.app).turned(from));
            }
            assert_eq!(turned > 0.05, lead == 9, "lead {lead}: turned {turned} before the second finger");
            // The second finger lands, and the view is where the first one found it.
            let a = a + step * f32::from(lead - 1);
            frame(&mut h, &mut input, &mut time, vec![finger(2, TouchPhase::Start, b)]);
            let joined = Look::of(&h.app);
            assert!(joined.turned(from) < 1e-9 && joined.carried(from) < 1e-9, "lead {lead}: {joined:?} from {from:?}");
            // Both go on together, and lift.
            for n in 1..=10_u16 {
                let on = step * f32::from(n);
                frame(&mut h, &mut input, &mut time, vec![finger(1, TouchPhase::Move, a + on), finger(2, TouchPhase::Move, b + on)]);
            }
            let (a, b) = (a + step * 10.0, b + step * 10.0);
            frame(&mut h, &mut input, &mut time, vec![finger(1, TouchPhase::End, a)]);
            frame(&mut h, &mut input, &mut time, vec![finger(2, TouchPhase::End, b)]);
            for _ in 0..3 {
                frame(&mut h, &mut input, &mut time, Vec::new());
            }
            let look = Look::of(&h.app);
            let expected = f64::from((step * 10.0).length()) * 2.0 * from.half_height / f64::from(view.height());
            assert!(look.turned(from) < 1e-9, "lead {lead}: turned {}", look.turned(from));
            assert!((look.carried(from) - expected).abs() <= expected * 0.02, "lead {lead}: carried {} of {expected}", look.carried(from));
            assert!((look.zoomed(from) - 1.0).abs() < 1e-6, "lead {lead}: zoomed {}", look.zoomed(from));
            assert_eq!(h.app.viewport.nav, None, "lead {lead}");
            assert!(h.app.session.selection.is_empty(), "lead {lead}: {:?}", h.app.session.selection);
        }
    }

    #[test]
    fn a_second_finger_long_after_the_first_leaves_the_turn_the_first_one_made() {
        let mut h = sample();
        let mut input = Input::default();
        let mut time = 10.0;
        let view = h.app.viewport.rect.unwrap();
        let from = Look::of(&h.app);
        let a = view.center() - vec2(60.0, 0.0);
        frame(&mut h, &mut input, &mut time, vec![finger(1, TouchPhase::Start, a)]);
        // Two thirds of a second of turning: longer than a second finger may be late by.
        for n in 1..=40_u16 {
            frame(&mut h, &mut input, &mut time, vec![finger(1, TouchPhase::Move, a + vec2(3.0, 0.0) * f32::from(n))]);
        }
        let turned = Look::of(&h.app).turned(from);
        assert!(turned > 0.2, "{turned}");
        frame(&mut h, &mut input, &mut time, vec![finger(2, TouchPhase::Start, view.center() + vec2(100.0, 0.0))]);
        frame(&mut h, &mut input, &mut time, Vec::new());
        assert!((Look::of(&h.app).turned(from) - turned).abs() < 1e-9, "the turn was meant and stays");
        assert_eq!(h.app.viewport.nav, None);
    }

    #[test]
    fn no_self_test_runs_unless_the_environment_asks() {
        // Neither variable is set where tests run.
        assert!(std::env::var(DEMO).is_err() && std::env::var(TOUCH_DEMO).is_err());
        let input = Input::from_env();
        assert!(!input.busy());
        assert!(Input { demo: Some(0), ..Default::default() }.busy());
        // (the Pencil's, the fingers'): one at a time, and the fingers' when both are asked for.
        assert_eq!(asked(None, None), (false, false));
        assert_eq!(asked(Some("ring"), None), (true, false));
        assert_eq!(asked(Some("ring"), Some("off")), (true, false));
        assert_eq!(asked(None, Some("camera")), (false, true));
        assert_eq!(asked(Some("ring"), Some("camera")), (false, true));
        assert_eq!(asked(Some("off"), Some("camera")), (false, true));
        assert_eq!(asked(Some("1"), Some("yes")), (false, false));
    }

    fn pencil_button(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }
}
