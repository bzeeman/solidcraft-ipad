//! Fingers and the Apple Pencil's tip on the screen, in the iPad spike.
//!
//! The window layer reports every touch, and makes the first one down the mouse's left button
//! whatever it is: a finger selected and drew as the Pencil does, and a second finger did
//! nothing. [`Touches::frame`] takes that pointer out of a frame's input and puts back the one
//! this app wants, so that the Pencil points and fingers move the view:
//!
//! - the Pencil's tip is the left button, as before. While it is on the screen fingers do
//!   nothing (a hand resting beside it), and they stay dead until they lift. The pointer stays
//!   where the tip came down until the tip has slid more than [`PENCIL_SLOP`] from there: a tip
//!   on glass slides a millimetre or so on an ordinary tap, and egui's own 6 points let that
//!   drag a part across the model. Then it follows the tip, catching up at once, so the point
//!   grabbed stays under the tip. A still tip moves nothing however long it rests;
//! - one finger that moves more than [`SLOP`] drags with the left button from where it came
//!   down: the press there in one frame, the way to the finger in the next, so that what is
//!   pressed is what the finger came down on. Over the model that turns the view, because the
//!   shell lights the navigation bar's Orbit mode for as long as the drag lasts; in a list it
//!   scrolls, on a slider it slides;
//! - one finger lifted before it moved that far, within [`HOLD`], is a click where it came down;
//! - one finger held that still for [`HOLD`] is held ([`Heard::held`], which the shell marks):
//!   each quick, still tap of another finger meanwhile is a Shift click where it taps, and if
//!   none came, lifting the held finger is a right click where it came down (the context menus
//!   open on it). A second finger that moves instead pans and pinches with it, as below. A
//!   second finger's quick, still tap before the hold is a Shift click too, and the first is
//!   then held as if its time were up;
//! - two fingers are a trackpad's two-finger scroll and pinch, which the camera already pans and
//!   zooms with and lists scroll with. The pointer sits between the fingers meanwhile, so the
//!   zoom is about that point. When one lifts, the other waits for a second and does nothing
//!   alone;
//! - a third finger does nothing.
//!
//! Two fingers meant to land together seldom do, and the first is often moving already. On the
//! iPad that turned the view a few degrees before every third two-finger gesture. So a finger
//! begins no drag in its first [`SETTLE`] seconds on the screen, however far it goes: a second
//! finger that lands in them finds no drag to end. And a second finger (or the Pencil's tip)
//! that lands within [`JOIN`] of a drag's beginning takes that drag back
//! ([`Heard::taken_back`]): the shell puts the view back where the drag found it.
//!
//! A click is read where the pointer is, so the pointer stays where a finger or the tip let go
//! for the rest of that frame and leaves the screen in the next. (The window layer takes it away
//! in the frame of the release, and a tap on the model then selected nothing.)
//!
//! A touch is the Pencil's when it reports a force: an iPad's screen measures none for fingers.
//!
//! Nothing here is specific to iOS: it reads and writes egui's input only, so it is tested on any
//! platform.

use std::collections::VecDeque;

use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2, Rect, TouchDeviceId, TouchId, TouchPhase, Vec2, vec2};

/// A finger that moves further than this (points) from where it came down drags; one lifted
/// before that is a tap. More than egui's own click distance (6), so that the release of a drag
/// is never a click.
pub const SLOP: f32 = 10.0;
/// A finger held this long (seconds) without moving [`SLOP`] is a right click where it came
/// down; one lifted sooner is a click. Less than egui's click time (0.8), so a tap is always a
/// click. Ben's taps on the iPad took 0.05 to 0.17 s.
pub const HOLD: f64 = 0.5;
/// A finger begins no drag in its first moments on the screen (seconds), however far it moves:
/// the second finger of a pair lands within them more often than not. A guess until the iPad's
/// log says how far apart Ben's two fingers land (the note of a second finger gives the time).
pub const SETTLE: f64 = 0.07;
/// A second finger, or the Pencil's tip, that lands this soon (seconds) after a finger's drag
/// began takes the drag back: the two were one gesture, begun a finger at a time. Counted from
/// the drag's beginning, not the finger's: a finger (or a hand) that has rested a while and
/// starts to slide as the other comes down is the same mistake.
pub const JOIN: f64 = 0.4;
/// Two fingers closer together than this (points) do not zoom: the ratio of two small distances
/// is noise.
const MIN_SPREAD: f32 = 24.0;
/// The most one movement of a finger may zoom by, either way.
const MAX_ZOOM_STEP: f32 = 2.0;
/// The Pencil's tip moves the pointer only once it is further than this (points) from where it
/// came down; before that the pointer stays there, and a lift there is a click there. About
/// 3 mm on the 13-inch iPad (264 pixels per inch, 2 pixels a point). Ben chose a bigger dead
/// zone for the tip, 2026-11-25, after it moved parts by accident.
pub const PENCIL_SLOP: f32 = 16.0;
/// The dead touches remembered at once; a stream that never lifts them loses the oldest.
const MAX_IGNORED: usize = 16;
/// The readout keeps this many of the latest gestures.
const NOTES: usize = 3;

/// A touch and where it is now.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Touch {
    id: TouchId,
    at: Pos2,
}

/// What the fingers are doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// No finger counts.
    Idle,
    /// One finger is down and has not moved [`SLOP`] from `origin` yet.
    Waiting { finger: Touch, origin: Pos2, since: f64 },
    /// One finger drags with the left button, pressed at `origin` at the time `since`. The press
    /// has a frame to itself: until `shown`, the app has not seen the pointer leave `origin`.
    Drag { finger: Touch, origin: Pos2, since: f64, shown: bool },
    /// Two fingers scroll and pinch; how far and how much so far, since when and from where
    /// (the point between them), for the readout and the log.
    /// `tap` holds where each came down while the second (`b`) may yet be a Shift tap: they
    /// began one finger at a time and neither has gone [`SLOP`] since.
    Two { a: Touch, b: Touch, moved: Vec2, zoomed: f32, since: f64, from: Pos2, tap: Option<(Pos2, Pos2)> },
    /// One of two fingers lifted: the other waits for a second one, and alone does nothing.
    Rest { finger: Touch },
    /// One finger held still for [`HOLD`] at `origin`. `tap` is another finger down since the
    /// time given, where it came down; `shifted` says a tap has Shift-clicked already, so the
    /// lift is no right click.
    Held { finger: Touch, origin: Pos2, tap: Option<(Touch, Pos2, f64)>, shifted: bool },
}

/// What [`Touches::frame`] heard in a frame's input.
#[derive(Debug, Default)]
pub struct Heard {
    /// A finger's drag holds the left button in this frame (the frame of its release included),
    /// pressed at this point.
    pub drag: Option<Pos2>,
    /// The pointer is a finger's in this frame (or between two of them), not the Pencil's.
    pub fingers: bool,
    /// The drag that ends in this frame was the first of two fingers that came down one after
    /// the other, or a hand that came down before the Pencil's tip: what it did to the view is
    /// to be undone.
    pub taken_back: bool,
    /// Look again after this many seconds even if no input arrives: a press or a release has a
    /// frame to itself and what follows it is owed, a drag has ended and the shell has
    /// something to undo, or a finger's first moments are over and its drag may begin.
    pub wake: Option<f64>,
    /// The gestures that ended in this frame, for the log.
    pub notes: Vec<String>,
    /// A finger is held (see [`HOLD`]) here: taps of another are Shift clicks.
    pub held: Option<Pos2>,
}

impl Heard {
    /// Look again after `after` seconds at the latest.
    fn wake(&mut self, after: f64) {
        self.wake = Some(self.wake.map_or(after, |wake| wake.min(after)));
    }
}

/// The touches on the screen, from one frame to the next.
#[derive(Debug)]
pub struct Touches {
    mode: Mode,
    /// The Pencil's tip, while it is on the screen.
    pencil: Option<Touch>,
    /// Where the tip came down, while it has not gone [`PENCIL_SLOP`] from there.
    tip_origin: Option<Pos2>,
    /// Touches that do nothing until they lift.
    ignored: Vec<TouchId>,
    /// A tap's (or a hold's) release, owed to the next frame: the app gets its press and its
    /// release a frame apart, as from a finger. Modifiers held for the click are let go after.
    release: Option<(Pos2, PointerButton, Modifiers)>,
    /// The pointer let go in the last frame and nothing has moved it since: it leaves the
    /// screen in this one.
    gone: bool,
    /// A Shift click was let go in the last frame: Shift goes in this one. egui reads a click on
    /// its release, with the frame's last modifiers, so they must outlast the release's frame.
    unshift: bool,
    notes: VecDeque<String>,
    now: f64,
}

impl Default for Touches {
    fn default() -> Self {
        Self { mode: Mode::Idle, pencil: None, tip_origin: None, ignored: Vec::new(), release: None, gone: false, unshift: false, notes: VecDeque::new(), now: 0.0 }
    }
}

fn button(at: Pos2, pressed: bool) -> Event {
    press(at, PointerButton::Primary, pressed)
}

fn press(at: Pos2, button: PointerButton, pressed: bool) -> Event {
    press_with(at, button, pressed, Modifiers::NONE)
}

fn press_with(at: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos: at, button, pressed, modifiers }
}

/// A step of a trackpad's two-finger scroll, `delta` points the way the fingers went.
fn wheel(phase: TouchPhase, delta: Vec2) -> Event {
    Event::MouseWheel { source: MouseWheelSource::Trackpad, unit: MouseWheelUnit::Point, delta, modifiers: Modifiers::NONE, phase }
}

fn between(a: Pos2, b: Pos2) -> Pos2 {
    a + (b - a) * 0.5
}

/// A frame's input as the app will get it.
struct Out {
    events: Vec<Event>,
    /// The last thing the pointer did was to let go of the button, by our doing.
    let_go: bool,
}

impl Out {
    /// An event of ours.
    fn push(&mut self, event: Event) {
        match event {
            Event::PointerButton { pressed, .. } => self.let_go = !pressed,
            Event::PointerMoved(_) | Event::PointerGone => self.let_go = false,
            _ => {}
        }
        self.events.push(event);
    }

    fn extend<const N: usize>(&mut self, events: [Event; N]) {
        for event in events {
            self.push(event);
        }
    }

    /// An event of the window layer's that stays.
    fn pass(&mut self, event: Event) {
        if matches!(event, Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone) {
            self.let_go = false;
        }
        self.events.push(event);
    }
}

/// Move the pointer to `at`, unless that is nowhere (two fingers a world apart).
fn point(out: &mut Out, at: Pos2) {
    if at.is_finite() {
        out.push(Event::PointerMoved(at));
    }
}

impl Touches {
    /// Read one frame's input before anything else does: the pointer the window layer made of
    /// the touches goes out, and the one described in the module's notes comes in. The touches
    /// themselves stay in the input (egui scrolls a list by a drag only once it has seen one).
    pub fn frame(&mut self, raw: &mut egui::RawInput) -> Heard {
        let now = raw.time.filter(|time| time.is_finite()).unwrap_or(self.now);
        self.now = now;
        let mut heard = Heard::default();
        let mut out = Out { events: Vec::with_capacity(raw.events.len() + 4), let_go: false };
        if std::mem::take(&mut self.gone) {
            out.push(Event::PointerGone);
        }
        if std::mem::take(&mut self.unshift) {
            out.push(Event::ModifiersChanged(Modifiers::NONE));
        }
        self.let_go(&mut out);
        match self.mode {
            Mode::Drag { finger, origin, since, shown } => {
                heard.drag = Some(origin);
                heard.fingers = true;
                if !shown {
                    // The press was the last frame's; now the pointer goes where the finger is.
                    point(&mut out, finger.at);
                    self.mode = Mode::Drag { finger, origin, since, shown: true };
                }
            }
            Mode::Waiting { .. } | Mode::Two { .. } => heard.fingers = true,
            Mode::Held { .. } => heard.fingers = true,
            Mode::Idle | Mode::Rest { .. } => {}
        }

        // The window layer follows each touch with the pointer it makes of it.
        let mut emulated = false;
        for event in std::mem::take(&mut raw.events) {
            let touch = match &event {
                Event::Touch { id, phase, pos, force, .. } => Some((Touch { id: *id, at: *pos }, *phase, force.is_some())),
                _ => None,
            };
            if let Some((touch, phase, pencil)) = touch {
                emulated = true;
                out.pass(event);
                self.touch(touch, phase, pencil, &mut out, &mut heard);
            } else if emulated && matches!(event, Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone) {
                // Ours replaces it.
            } else {
                emulated = false;
                out.pass(event);
            }
        }
        self.ripen(&mut out, &mut heard);
        if let Mode::Held { finger, .. } = self.mode {
            heard.held = Some(finger.at);
        }
        if out.let_go {
            // The click is read where the pointer is: it stays for this frame.
            self.gone = true;
            heard.wake(0.0);
        }
        raw.events = out.events;
        heard
    }

    /// What is left (seconds) of the first moments of a finger down since `since`, while any is.
    /// A clock that ran backwards leaves none.
    fn settling(&self, since: f64) -> Option<f64> {
        let left = SETTLE - (self.now - since);
        (left > 0.0 && left <= SETTLE).then_some(left)
    }

    /// At the end of a frame's input: a finger still alone and further than [`SLOP`] from where
    /// it came down begins its drag, once its first moments are over; one still within it after
    /// [`HOLD`] right-clicks there.
    fn ripen(&mut self, out: &mut Out, heard: &mut Heard) {
        let Mode::Waiting { finger, origin, since } = self.mode else {
            return;
        };
        if origin.distance(finger.at) <= SLOP {
            let held = self.now - since;
            if !(0.0..HOLD).contains(&held) {
                // Held still (or a clock that ran backwards): held, until it lifts or another
                // finger taps.
                self.mode = Mode::Held { finger, origin, tap: None, shifted: false };
                heard.fingers = true;
                heard.notes.push(format!("finger held at {:.0}, {:.0}", origin.x, origin.y));
            } else {
                heard.wake(HOLD - held);
            }
            return;
        }
        if let Some(left) = self.settling(since) {
            heard.wake(left);
            return;
        }
        // The press is where the finger came down, as a mouse's would be, and the way from
        // there is the next frame's: egui presses what is under the pointer at the end of the
        // frame.
        out.extend([Event::PointerMoved(origin), button(origin, true)]);
        self.mode = Mode::Drag { finger, origin, since: self.now, shown: false };
        heard.drag = Some(origin);
        heard.fingers = true;
        heard.wake(0.0);
    }

    /// Whether a drag begun at `since` is joined soon enough (see [`JOIN`]) to be taken back,
    /// and how long it has lasted.
    fn joined(&self, since: f64) -> (bool, f64) {
        let held = self.now - since;
        ((0.0..=JOIN).contains(&held), held)
    }

    /// What the fingers are doing and their latest gestures, a line each, for the spike's
    /// readout.
    pub fn readout(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.notes.len() + 1);
        match self.mode {
            _ if self.pencil.is_some() => lines.push("touch: the Pencil's tip".to_owned()),
            Mode::Idle => {}
            Mode::Waiting { .. } => lines.push("touch: one finger, not moved yet".to_owned()),
            Mode::Drag { .. } => lines.push("touch: one finger dragging".to_owned()),
            Mode::Held { .. } => lines.push("touch: one finger held: taps of another are Shift clicks".to_owned()),
            Mode::Two { moved, zoomed, .. } => lines.push(format!("touch: two fingers, moved {:.0}, {:.0} pt, zoom x{zoomed:.2}", moved.x, moved.y)),
            Mode::Rest { .. } => lines.push("touch: one of two fingers left".to_owned()),
        }
        lines.extend(self.notes.iter().cloned());
        lines
    }

    fn note(&mut self, text: String, heard: &mut Heard) {
        if self.notes.len() >= NOTES {
            self.notes.pop_front();
        }
        self.notes.push_back(text.clone());
        heard.notes.push(text);
    }

    /// The release a tap still owes.
    fn let_go(&mut self, out: &mut Out) {
        if let Some((at, which, modifiers)) = self.release.take() {
            out.push(press_with(at, which, false, modifiers));
            // Let go of the modifiers in the next frame (the pointer's going wakes it).
            self.unshift |= !modifiers.is_none();
        }
    }

    fn ignore(&mut self, id: TouchId) {
        if self.ignored.len() >= MAX_IGNORED {
            self.ignored.remove(0);
        }
        self.ignored.push(id);
    }

    fn knows(&self, id: TouchId) -> bool {
        let finger = match self.mode {
            Mode::Idle => false,
            Mode::Waiting { finger, .. } | Mode::Drag { finger, .. } | Mode::Rest { finger } => finger.id == id,
            Mode::Two { a, b, .. } => a.id == id || b.id == id,
            Mode::Held { finger, tap, .. } => finger.id == id || tap.is_some_and(|(tap, ..)| tap.id == id),
        };
        finger || self.pencil.is_some_and(|pencil| pencil.id == id) || self.ignored.contains(&id)
    }

    fn touch(&mut self, touch: Touch, phase: TouchPhase, pencil: bool, out: &mut Out, heard: &mut Heard) {
        let at = touch.at.is_finite().then_some(touch.at);
        match phase {
            TouchPhase::Start if !self.knows(touch.id) => match at {
                Some(_) if pencil => self.pencil_down(touch, out, heard),
                Some(_) => self.finger_down(touch, out, heard),
                // Down nowhere: it does nothing until it lifts.
                None => self.ignore(touch.id),
            },
            // A touch said to begin twice has moved.
            TouchPhase::Start | TouchPhase::Move => {
                if at.is_some() {
                    self.moved(touch, out, heard);
                }
            }
            TouchPhase::End => self.lifted(touch.id, at, true, out, heard),
            TouchPhase::Cancel => self.lifted(touch.id, at, false, out, heard),
        }
    }

    fn pencil_down(&mut self, touch: Touch, out: &mut Out, heard: &mut Heard) {
        self.let_go(out);
        // What the fingers were doing ends here, and they do nothing more.
        match std::mem::replace(&mut self.mode, Mode::Idle) {
            Mode::Idle => {}
            Mode::Waiting { finger, .. } | Mode::Rest { finger } => self.ignore(finger.id),
            Mode::Held { finger, tap, .. } => {
                self.ignore(finger.id);
                if let Some((tap, ..)) = tap {
                    self.ignore(tap.id);
                }
                self.note("a held finger, then the Pencil came down: nothing done".to_owned(), heard);
            }
            Mode::Drag { finger, origin, since, .. } => {
                out.push(button(finger.at, false));
                // The tip's press must not be read as the drag's: the shell lets go of the
                // drag's mode before this frame.
                heard.drag = None;
                self.ignore(finger.id);
                // A hand that came down a moment before the tip was no drag.
                let (young, held) = self.joined(since);
                heard.taken_back = young;
                let undone = if young { ": taken back" } else { "" };
                self.note(
                    format!("one finger dragged {:.0} pt in {held:.2} s, then the Pencil came down{undone}", origin.distance(finger.at)),
                    heard,
                );
            }
            Mode::Two { a, b, moved, zoomed, .. } => {
                out.push(wheel(TouchPhase::End, Vec2::ZERO));
                self.ignore(a.id);
                self.ignore(b.id);
                self.note(format!("two fingers moved {:.0}, {:.0} pt and zoomed x{zoomed:.2}, then the Pencil came down", moved.x, moved.y), heard);
            }
        }
        if let Some(other) = self.pencil.take() {
            // Two tips at once cannot be; the first is let go.
            out.push(button(other.at, false));
            self.ignore(other.id);
        }
        heard.fingers = false;
        out.extend([Event::PointerMoved(touch.at), button(touch.at, true)]);
        self.pencil = Some(touch);
        self.tip_origin = Some(touch.at);
    }

    fn finger_down(&mut self, touch: Touch, out: &mut Out, heard: &mut Heard) {
        if self.pencil.is_some() {
            // A hand resting beside the tip.
            self.ignore(touch.id);
            return;
        }
        heard.fingers = true;
        match self.mode {
            Mode::Idle => {
                self.let_go(out);
                out.push(Event::PointerMoved(touch.at));
                self.mode = Mode::Waiting { finger: touch, origin: touch.at, since: self.now };
            }
            Mode::Waiting { finger, origin, since } => {
                // For the log only: how far apart in time two fingers land is what SETTLE and
                // JOIN are guesses at.
                heard.notes.push(format!(
                    "a second finger came down {:.2} s after the first, which had moved {:.0} pt",
                    self.now - since,
                    origin.distance(finger.at)
                ));
                self.two_from(finger, touch, Some((origin, touch.at)), out);
            }
            Mode::Rest { finger } => self.two(finger, touch, out),
            Mode::Drag { finger, origin, since, .. } => {
                // Past the slop, so this release is no click.
                out.push(button(finger.at, false));
                heard.drag = Some(origin);
                heard.wake(0.0);
                let (young, held) = self.joined(since);
                heard.taken_back = young;
                let undone = if young { ": taken back" } else { "" };
                self.note(format!("one finger dragged {:.0} pt in {held:.2} s, then a second came down{undone}", origin.distance(finger.at)), heard);
                self.two(finger, touch, out);
            }
            Mode::Held { finger, origin, tap: None, shifted } => {
                out.push(Event::PointerMoved(touch.at));
                self.mode = Mode::Held { finger, origin, tap: Some((touch, touch.at, self.now)), shifted };
            }
            Mode::Two { .. } | Mode::Held { .. } => self.ignore(touch.id),
        }
    }

    /// Two fingers begin a scroll, with the pointer between them.
    fn two(&mut self, a: Touch, b: Touch, out: &mut Out) {
        self.two_from(a, b, None, out);
    }

    /// As [`Self::two`], with `b` perhaps a Shift tap: `tap` is where each came down.
    fn two_from(&mut self, a: Touch, b: Touch, tap: Option<(Pos2, Pos2)>, out: &mut Out) {
        let from = between(a.at, b.at);
        point(out, from);
        out.push(wheel(TouchPhase::Start, Vec2::ZERO));
        self.mode = Mode::Two { a, b, moved: Vec2::ZERO, zoomed: 1.0, since: self.now, from, tap };
    }

    fn moved(&mut self, touch: Touch, out: &mut Out, heard: &mut Heard) {
        if let Some(pencil) = &mut self.pencil
            && pencil.id == touch.id
        {
            pencil.at = touch.at;
            match self.tip_origin {
                Some(origin) if origin.distance(touch.at) <= PENCIL_SLOP => {}
                _ => {
                    self.tip_origin = None;
                    out.push(Event::PointerMoved(touch.at));
                }
            }
            return;
        }
        match self.mode {
            Mode::Waiting { finger, origin, since } if finger.id == touch.id => {
                // Whether it drags is decided when the frame's input is read ([`Self::ripen`]):
                // a second finger may be next in it.
                heard.fingers = true;
                out.push(Event::PointerMoved(touch.at));
                self.mode = Mode::Waiting { finger: touch, origin, since };
            }
            Mode::Drag { finger, origin, since, shown } if finger.id == touch.id => {
                heard.fingers = true;
                if shown {
                    out.push(Event::PointerMoved(touch.at));
                }
                self.mode = Mode::Drag { finger: touch, origin, since, shown };
            }
            Mode::Two { a, b, mut moved, mut zoomed, since, from, mut tap } if a.id == touch.id || b.id == touch.id => {
                heard.fingers = true;
                let (centre, spread) = (between(a.at, b.at), a.at.distance(b.at));
                let (a, b) = if a.id == touch.id { (touch, b) } else { (a, touch) };
                if tap.is_some_and(|(at_a, at_b)| at_a.distance(a.at) > SLOP || at_b.distance(b.at) > SLOP) {
                    tap = None;
                }
                let (centre_now, spread_now) = (between(a.at, b.at), a.at.distance(b.at));
                point(out, centre_now);
                let delta = centre_now - centre;
                if delta.is_finite() && delta != Vec2::ZERO {
                    out.push(wheel(TouchPhase::Move, delta));
                    moved += delta;
                }
                if spread >= MIN_SPREAD && spread_now >= MIN_SPREAD {
                    let factor = (spread_now / spread).clamp(1.0 / MAX_ZOOM_STEP, MAX_ZOOM_STEP);
                    if factor.is_finite() && factor != 1.0 {
                        out.push(Event::Zoom(factor));
                        zoomed *= factor;
                    }
                }
                self.mode = Mode::Two { a, b, moved, zoomed, since, from, tap };
            }
            Mode::Rest { finger } if finger.id == touch.id => self.mode = Mode::Rest { finger: touch },
            Mode::Held { finger, origin, tap, shifted } if finger.id == touch.id => {
                heard.fingers = true;
                if origin.distance(touch.at) <= SLOP {
                    self.mode = Mode::Held { finger: touch, origin, tap, shifted };
                } else if let Some((other, ..)) = tap {
                    // Both move: a pan or a pinch after all.
                    self.two(touch, other, out);
                } else {
                    // The held finger slid off: it does nothing more.
                    self.mode = Mode::Idle;
                    self.ignore(touch.id);
                    out.push(Event::PointerGone);
                    self.note("a held finger slid away: nothing done".to_owned(), heard);
                }
            }
            Mode::Held { finger, origin, tap: Some((other, from, since)), shifted } if other.id == touch.id => {
                heard.fingers = true;
                if from.distance(touch.at) <= SLOP {
                    self.mode = Mode::Held { finger, origin, tap: Some((touch, from, since)), shifted };
                } else {
                    // Not a tap: the two pan and pinch.
                    self.two(finger, touch, out);
                }
            }
            _ => {}
        }
    }

    /// A touch is over: lifted (`ended`), or taken back by the system.
    fn lifted(&mut self, id: TouchId, at: Option<Pos2>, ended: bool, out: &mut Out, heard: &mut Heard) {
        if let Some(pencil) = self.pencil
            && pencil.id == id
        {
            let at = at.unwrap_or(pencil.at);
            // Lifted within the dead zone: let go where it came down, so a tap that slid a
            // little is a click there and not a drag.
            let at = match self.tip_origin.take() {
                Some(origin) if origin.distance(at) <= PENCIL_SLOP => origin,
                _ => at,
            };
            out.extend([Event::PointerMoved(at), button(at, false)]);
            self.pencil = None;
            // For the log only: the readout's few lines are the fingers'.
            heard.notes.push(format!("the Pencil's tip lifted at {:.0}, {:.0}", at.x, at.y));
            return;
        }
        match self.mode {
            Mode::Waiting { finger, origin, since } if finger.id == id => {
                heard.fingers = true;
                self.mode = Mode::Idle;
                let held = self.now - since;
                // Past the slop with no drag begun: it went that far in its first moments, or
                // within one frame's input. A flick too short to be meant.
                let went = origin.distance(at.unwrap_or(finger.at));
                if ended && went <= SLOP && (0.0..HOLD).contains(&held) {
                    out.extend([Event::PointerMoved(origin), button(origin, true)]);
                    self.release = Some((origin, PointerButton::Primary, Modifiers::NONE));
                    heard.wake(0.0);
                    self.note(format!("finger tap at {:.0}, {:.0} after {held:.2} s", origin.x, origin.y), heard);
                } else {
                    out.push(Event::PointerGone);
                    let why = if !ended {
                        "taken back by the system".to_owned()
                    } else if went > SLOP {
                        format!("lifted {went:.0} pt away after {held:.2} s, before a drag began")
                    } else {
                        format!("lifted after {held:.1} s without moving")
                    };
                    self.note(format!("finger {why}: nothing done"), heard);
                }
            }
            Mode::Drag { finger, origin, since, .. } if finger.id == id => {
                heard.fingers = true;
                heard.drag = Some(origin);
                heard.wake(0.0);
                let at = at.unwrap_or(finger.at);
                out.extend([Event::PointerMoved(at), button(at, false)]);
                self.mode = Mode::Idle;
                self.note(
                    format!("one finger dragged {:.0} pt from {:.0}, {:.0} in {:.2} s", origin.distance(at), origin.x, origin.y, self.now - since),
                    heard,
                );
            }
            Mode::Two { a, b: second, tap: Some((origin, at_b)), since, .. }
                if second.id == id && ended && (0.0..HOLD).contains(&(self.now - since)) && at.is_none_or(|at| at.distance(at_b) <= SLOP) =>
            {
                // The second finger tapped while the first stayed: a Shift click where it came
                // down, and the first is held.
                heard.fingers = true;
                let shift = Modifiers::SHIFT;
                out.extend([
                    wheel(TouchPhase::End, Vec2::ZERO),
                    Event::ModifiersChanged(shift),
                    Event::PointerMoved(at_b),
                    press_with(at_b, PointerButton::Primary, true, shift),
                ]);
                self.release = Some((at_b, PointerButton::Primary, shift));
                heard.wake(0.0);
                self.mode = Mode::Held { finger: a, origin, tap: None, shifted: true };
                self.note(format!("Shift tap at {:.0}, {:.0} while the first finger stayed", at_b.x, at_b.y), heard);
            }
            Mode::Two { a, b, moved, zoomed, since, from, .. } if a.id == id || b.id == id => {
                heard.fingers = true;
                out.extend([wheel(TouchPhase::End, Vec2::ZERO), Event::PointerGone]);
                self.mode = Mode::Rest { finger: if a.id == id { b } else { a } };
                self.note(
                    format!(
                        "two fingers moved {:.0}, {:.0} pt and zoomed x{zoomed:.2} from {:.0}, {:.0} in {:.2} s",
                        moved.x,
                        moved.y,
                        from.x,
                        from.y,
                        self.now - since
                    ),
                    heard,
                );
            }
            Mode::Rest { finger } if finger.id == id => self.mode = Mode::Idle,
            Mode::Held { finger, origin, tap, shifted } if finger.id == id => {
                heard.fingers = true;
                if let Some((other, ..)) = tap {
                    // The tapping finger is left alone: it does nothing.
                    self.mode = Mode::Rest { finger: other };
                    out.push(Event::PointerGone);
                    self.note("a held finger lifted while another was down: nothing done".to_owned(), heard);
                } else if ended && !shifted {
                    self.mode = Mode::Idle;
                    out.extend([Event::PointerMoved(origin), press(origin, PointerButton::Secondary, true)]);
                    self.release = Some((origin, PointerButton::Secondary, Modifiers::NONE));
                    heard.wake(0.0);
                    self.note(format!("finger held at {:.0}, {:.0}, then lifted: right click", origin.x, origin.y), heard);
                } else {
                    self.mode = Mode::Idle;
                    out.push(Event::PointerGone);
                    let why = if shifted { "after Shift clicks" } else { "taken back by the system" };
                    self.note(format!("a held finger lifted {why}: nothing more done"), heard);
                }
            }
            Mode::Held { finger, origin, tap: Some((other, from, since)), shifted } if other.id == id => {
                heard.fingers = true;
                let held = self.now - since;
                let went = from.distance(at.unwrap_or(other.at));
                if ended && went <= SLOP && (0.0..HOLD).contains(&held) {
                    let shift = Modifiers::SHIFT;
                    out.extend([Event::ModifiersChanged(shift), Event::PointerMoved(from), press_with(from, PointerButton::Primary, true, shift)]);
                    self.release = Some((from, PointerButton::Primary, shift));
                    heard.wake(0.0);
                    self.mode = Mode::Held { finger, origin, tap: None, shifted: true };
                    self.note(format!("Shift tap at {:.0}, {:.0} while a finger was held", from.x, from.y), heard);
                } else {
                    self.mode = Mode::Held { finger, origin, tap: None, shifted };
                    out.push(Event::PointerGone);
                    self.note("a second finger lifted, too slow or too far for a tap: nothing done".to_owned(), heard);
                }
            }
            _ => self.ignored.retain(|ignored| *ignored != id),
        }
    }
}

/// The self-test's fingers move this many times in each part that moves.
const DEMO_MOVES: u16 = 10;
/// The self-test's two fingers come down this far apart, in points.
const DEMO_APART: f32 = 120.0;
/// How far the self-test's two fingers carry the model, in points.
pub const DEMO_PAN: Vec2 = vec2(60.0, 30.0);
/// How much further apart the self-test's two fingers end than they began.
pub const DEMO_PINCH: f32 = 2.0;
/// How far the self-test's one finger drags, in points.
pub const DEMO_DRAG: f32 = 120.0;
/// The steps the self-test's parts begin on: a tap, two fingers carrying the model, two
/// spreading, one dragging.
const DEMO_TAP_FROM: u16 = 0;
const DEMO_PAN_FROM: u16 = 8;
const DEMO_PINCH_FROM: u16 = 26;
const DEMO_DRAG_FROM: u16 = 44;
/// The step by which the self-test's tap has clicked and let go.
pub const DEMO_TAP_DONE: u16 = DEMO_PAN_FROM - 1;
/// The step by which its two fingers have carried the model and lifted.
pub const DEMO_PAN_DONE: u16 = DEMO_PINCH_FROM - 1;
/// The step by which its two fingers have spread and lifted.
pub const DEMO_PINCH_DONE: u16 = DEMO_DRAG_FROM - 1;
/// The step by which its one finger has dragged and lifted: the last step.
pub const DEMO_DRAG_DONE: u16 = DEMO_DRAG_FROM + DEMO_MOVES + 5;

/// The self-test's fingers: the touches of `step` (one frame each) over a view at `view`, and
/// `None` once they are done. One finger taps the view's centre; two fingers carry the model
/// [`DEMO_PAN`] and lift; two fingers spread to [`DEMO_PINCH`] times as far apart and lift; one
/// finger drags [`DEMO_DRAG`] sideways and lifts. A simulator on a build machine has no fingers
/// of its own.
pub fn demo(step: u16, view: Rect) -> Option<Vec<Event>> {
    use TouchPhase::{End, Move, Start};
    if step > DEMO_DRAG_DONE {
        return None;
    }
    let centre = view.center();
    let touch = |id: u64, phase: TouchPhase, at: Pos2| Event::Touch { device_id: TouchDeviceId(0), id: TouchId(id), phase, pos: at, force: None };
    // How far through its moves a part is whose first move is step `first`, from 0 to 1.
    let through = |first: u16| (f32::from(step.saturating_sub(first) + 1) / f32::from(DEMO_MOVES)).min(1.0);
    // Two fingers `apart` points apart about a point `off` the view's centre.
    let pair = |apart: f32, off: Vec2| (centre + off - vec2(apart / 2.0, 0.0), centre + off + vec2(apart / 2.0, 0.0));
    // A part of two fingers from step `from`: one comes down, then the other, they move, and
    // they lift one after the other.
    let two = |from: u16, ids: (u64, u64), at: (Pos2, Pos2)| match step.checked_sub(from) {
        Some(0) => vec![touch(ids.0, Start, at.0)],
        Some(1) => vec![touch(ids.1, Start, at.1)],
        Some(part) if part < DEMO_MOVES + 2 => vec![touch(ids.0, Move, at.0), touch(ids.1, Move, at.1)],
        Some(part) if part == DEMO_MOVES + 2 => vec![touch(ids.0, End, at.0)],
        Some(part) if part == DEMO_MOVES + 3 => vec![touch(ids.1, End, at.1)],
        _ => Vec::new(),
    };
    let tap = match step.checked_sub(DEMO_TAP_FROM) {
        Some(0) => vec![touch(1, Start, centre)],
        Some(2) => vec![touch(1, End, centre)],
        _ => Vec::new(),
    };
    let moved = if step < DEMO_PAN_FROM + 2 { 0.0 } else { through(DEMO_PAN_FROM + 2) };
    let carried = two(DEMO_PAN_FROM, (2, 3), pair(DEMO_APART, DEMO_PAN * moved));
    let moved = if step < DEMO_PINCH_FROM + 2 { 0.0 } else { through(DEMO_PINCH_FROM + 2) };
    let spread = two(DEMO_PINCH_FROM, (4, 5), pair(DEMO_APART * (1.0 + (DEMO_PINCH - 1.0) * moved), Vec2::ZERO));
    let moved = if step < DEMO_DRAG_FROM + 1 { 0.0 } else { through(DEMO_DRAG_FROM + 1) };
    let at = centre + vec2(DEMO_DRAG * moved, 40.0);
    let dragged = match step.checked_sub(DEMO_DRAG_FROM) {
        Some(0) => vec![touch(6, Start, at)],
        Some(part) if part <= DEMO_MOVES => vec![touch(6, Move, at)],
        Some(part) if part == DEMO_MOVES + 1 => vec![touch(6, End, at)],
        _ => Vec::new(),
    };
    Some([tap, carried, spread, dragged].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finger(id: u64, phase: TouchPhase, at: Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(0), id: TouchId(id), phase, pos: at, force: None }
    }

    fn tip(id: u64, phase: TouchPhase, at: Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(0), id: TouchId(id), phase, pos: at, force: Some(0.3) }
    }

    /// One frame at `time` with `events`; what was heard, and the pointer, wheel and zoom the
    /// app would see (the touches themselves left out).
    fn frame(touches: &mut Touches, time: f64, events: Vec<Event>) -> (Heard, Vec<Event>) {
        let mut raw = egui::RawInput { time: Some(time), events, ..Default::default() };
        let heard = touches.frame(&mut raw);
        (heard, raw.events.into_iter().filter(|event| !matches!(event, Event::Touch { .. })).collect())
    }

    fn at(x: f32, y: f32) -> Pos2 {
        Pos2::new(x, y)
    }

    fn pressed(events: &[Event]) -> usize {
        events.iter().filter(|event| matches!(event, Event::PointerButton { pressed: true, .. })).count()
    }

    fn released(events: &[Event]) -> usize {
        events.iter().filter(|event| matches!(event, Event::PointerButton { pressed: false, .. })).count()
    }

    /// The wheel's movement and the zoom in `events`.
    fn scrolled(events: &[Event]) -> (Vec2, f32) {
        events.iter().fold((Vec2::ZERO, 1.0), |(moved, zoomed), event| match event {
            Event::MouseWheel { delta, phase: TouchPhase::Move, .. } => (moved + *delta, zoomed),
            Event::Zoom(factor) => (moved, zoomed * factor),
            _ => (moved, zoomed),
        })
    }

    fn phases(events: &[Event]) -> Vec<TouchPhase> {
        events.iter().filter_map(|event| if let Event::MouseWheel { phase, .. } = event { Some(*phase) } else { None }).collect()
    }

    #[test]
    fn a_finger_tap_is_a_click_where_it_came_down_with_its_release_a_frame_later() {
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        let (heard, seen) = frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        assert_eq!(seen, vec![Event::PointerMoved(down)], "no press until it is known to be a tap");
        assert!(heard.fingers && heard.drag.is_none());
        assert_eq!(heard.wake, Some(HOLD), "looked at again when it would be a hold");
        // It wobbles less than the slop, and lifts a little way off.
        let (_, seen) = frame(&mut touches, 1.05, vec![finger(1, TouchPhase::Move, at(304.0, 403.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(at(304.0, 403.0))]);
        let (heard, seen) = frame(&mut touches, 1.1, vec![finger(1, TouchPhase::End, at(305.0, 403.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(down), button(down, true)]);
        assert_eq!(heard.wake, Some(0.0), "the release needs a frame of its own");
        assert!(heard.drag.is_none());
        assert_eq!(heard.notes, vec!["finger tap at 300, 400 after 0.10 s".to_owned()]);
        // The release with the pointer still there (the app reads a click where the pointer
        // is), and the pointer's going a frame after that.
        let (heard, seen) = frame(&mut touches, 1.11, vec![]);
        assert_eq!(seen, vec![button(down, false)]);
        assert!(!heard.fingers);
        assert_eq!(heard.wake, Some(0.0));
        let (heard, seen) = frame(&mut touches, 1.12, vec![]);
        assert_eq!(seen, vec![Event::PointerGone]);
        assert!(heard.wake.is_none());
        let (_, seen) = frame(&mut touches, 1.13, vec![]);
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn a_finger_that_moves_past_the_slop_drags_from_where_it_came_down() {
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        frame(&mut touches, 0.9, vec![finger(1, TouchPhase::Start, down)]);
        let past = at(300.0 + SLOP + 1.0, 400.0);
        let further = at(330.0, 400.0);
        // The press has the frame to itself, however far the finger goes in it: the frame ends
        // with the pointer back where the finger came down, pressed.
        let (heard, seen) = frame(&mut touches, 1.02, vec![finger(1, TouchPhase::Move, past), finger(1, TouchPhase::Move, further)]);
        assert_eq!(seen, vec![Event::PointerMoved(past), Event::PointerMoved(further), Event::PointerMoved(down), button(down, true)]);
        assert_eq!(heard.drag, Some(down));
        assert_eq!(heard.wake, Some(0.0), "the way to the finger is the next frame's");
        // The next frame goes to the finger, with or without input, and is still the drag's.
        let (heard, seen) = frame(&mut touches, 1.03, vec![]);
        assert_eq!(seen, vec![Event::PointerMoved(further)]);
        assert_eq!(heard.drag, Some(down));
        assert!(heard.fingers && heard.wake.is_none());
        let (heard, seen) = frame(&mut touches, 1.035, vec![]);
        assert!(seen.is_empty());
        assert_eq!(heard.drag, Some(down));
        let far = at(420.0, 380.0);
        let (heard, seen) = frame(&mut touches, 1.04, vec![finger(1, TouchPhase::Move, far)]);
        assert_eq!(seen, vec![Event::PointerMoved(far)]);
        assert_eq!(heard.drag, Some(down));
        // The frame of the release is the drag's too; the one after is not.
        let (heard, seen) = frame(&mut touches, 1.05, vec![finger(1, TouchPhase::End, far)]);
        assert_eq!(seen, vec![Event::PointerMoved(far), button(far, false)]);
        assert_eq!(heard.drag, Some(down));
        assert_eq!(heard.wake, Some(0.0));
        let (heard, seen) = frame(&mut touches, 1.06, vec![]);
        assert_eq!(seen, vec![Event::PointerGone]);
        assert!(heard.drag.is_none() && !heard.fingers);
        assert!(touches.readout().iter().any(|line| line.contains("one finger dragged 122 pt from 300, 400 in 0.03 s")), "{:?}", touches.readout());
    }

    #[test]
    fn a_finger_begins_no_drag_in_its_first_moments_however_far_it_goes() {
        let mut touches = Touches::default();
        let (down, far) = (at(300.0, 400.0), at(340.0, 400.0));
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        let (heard, seen) = frame(&mut touches, 1.03, vec![finger(1, TouchPhase::Move, far)]);
        assert_eq!(seen, vec![Event::PointerMoved(far)], "the pointer follows it, unpressed");
        assert!(heard.fingers && heard.drag.is_none());
        assert!(heard.wake.is_some_and(|wake| (wake - (SETTLE - 0.03)).abs() < 1e-9), "it asks to be looked at when they are over: {:?}", heard.wake);
        // Looked at too early: still nothing, and the rest of the wait.
        let (heard, seen) = frame(&mut touches, 1.05, vec![]);
        assert!(seen.is_empty() && heard.drag.is_none(), "{seen:?}");
        assert!(heard.wake.is_some_and(|wake| (wake - (SETTLE - 0.05)).abs() < 1e-9), "{:?}", heard.wake);
        // The moments are over: the press where it came down, the way to the finger a frame on.
        let (heard, seen) = frame(&mut touches, 1.0 + SETTLE + 0.001, vec![]);
        assert_eq!(seen, vec![Event::PointerMoved(down), button(down, true)]);
        assert_eq!((heard.drag, heard.wake), (Some(down), Some(0.0)));
        let (heard, seen) = frame(&mut touches, 1.08, vec![]);
        assert_eq!(seen, vec![Event::PointerMoved(far)]);
        assert_eq!(heard.drag, Some(down));

        // One that is back within the slop by then drags nothing, and its lift is a tap.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.02, vec![finger(1, TouchPhase::Move, far)]);
        frame(&mut touches, 1.04, vec![finger(1, TouchPhase::Move, at(302.0, 400.0))]);
        let (heard, seen) = frame(&mut touches, 1.2, vec![]);
        assert!(seen.is_empty() && heard.drag.is_none(), "{seen:?}");
        assert!(heard.wake.is_some_and(|wake| (wake - (HOLD - 0.2)).abs() < 1e-9), "only the hold is waited for: {:?}", heard.wake);
        let (_, seen) = frame(&mut touches, 1.3, vec![finger(1, TouchPhase::End, at(302.0, 400.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(down), button(down, true)]);
    }

    #[test]
    fn a_finger_that_goes_far_and_lifts_before_a_drag_began_does_nothing() {
        let (down, far) = (at(300.0, 400.0), at(400.0, 400.0));
        // In its first moments.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.03, vec![finger(1, TouchPhase::Move, far)]);
        let (heard, seen) = frame(&mut touches, 1.05, vec![finger(1, TouchPhase::End, far)]);
        assert_eq!(seen, vec![Event::PointerGone]);
        assert!(heard.drag.is_none() && heard.wake.is_none());
        assert_eq!(heard.notes, vec!["finger lifted 100 pt away after 0.05 s, before a drag began: nothing done".to_owned()]);
        let (_, seen) = frame(&mut touches, 1.2, vec![]);
        assert!(seen.is_empty(), "nothing is owed: {seen:?}");
        // Later, but all of it in one frame's input.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        let (heard, seen) = frame(&mut touches, 1.5, vec![finger(1, TouchPhase::Move, far), finger(1, TouchPhase::End, far)]);
        assert_eq!(seen, vec![Event::PointerMoved(far), Event::PointerGone]);
        assert!(heard.drag.is_none());
        assert_eq!(touches.mode, Mode::Idle);
    }

    #[test]
    fn a_finger_held_still_then_lifted_is_a_right_click_where_it_came_down() {
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        let (heard, _) = frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        assert!(heard.wake.is_some_and(|wake| (wake - HOLD).abs() < 1e-9), "woken when the hold is due: {:?}", heard.wake);
        // A tremble within the slop is still holding.
        let (heard, seen) = frame(&mut touches, 1.3, vec![finger(1, TouchPhase::Move, at(304.0, 402.0))]);
        assert_eq!(pressed(&seen), 0);
        assert!(heard.wake.is_some_and(|wake| wake <= HOLD - 0.3 + 1e-9));
        // No input arrives; the wake comes and the finger is held, with nothing pressed yet.
        let (heard, seen) = frame(&mut touches, 1.0 + HOLD, vec![]);
        assert_eq!(pressed(&seen), 0, "{seen:?}");
        assert_eq!(heard.held, Some(at(304.0, 402.0)));
        assert!(heard.drag.is_none());
        let (heard, _) = frame(&mut touches, 2.5, vec![]);
        assert!(heard.held.is_some(), "held for as long as it stays");
        // It lifts: the right button where it came down, its release a frame later.
        let (heard, seen) = frame(&mut touches, 3.0, vec![finger(1, TouchPhase::End, at(304.0, 402.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(down), press(down, PointerButton::Secondary, true)]);
        assert_eq!(heard.wake, Some(0.0));
        let (_, seen) = frame(&mut touches, 3.02, vec![]);
        assert_eq!(seen, vec![press(down, PointerButton::Secondary, false)]);
        let (_, seen) = frame(&mut touches, 3.04, vec![]);
        assert_eq!(seen, vec![Event::PointerGone]);
        assert_eq!(touches.mode, Mode::Idle);
    }

    #[test]
    fn taps_of_another_finger_while_one_is_held_are_shift_clicks_and_the_lift_is_then_nothing() {
        let mut touches = Touches::default();
        let (down, there, again) = (at(100.0, 900.0), at(500.0, 400.0), at(520.0, 380.0));
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.0 + HOLD, vec![]);
        for (time, id, at) in [(2.0, 2, there), (3.0, 3, again)] {
            let (_, seen) = frame(&mut touches, time, vec![finger(id, TouchPhase::Start, at)]);
            assert_eq!(pressed(&seen), 0, "{seen:?}");
            let (heard, seen) = frame(&mut touches, time + 0.1, vec![finger(id, TouchPhase::End, at)]);
            assert_eq!(
                seen,
                vec![Event::ModifiersChanged(Modifiers::SHIFT), Event::PointerMoved(at), press_with(at, PointerButton::Primary, true, Modifiers::SHIFT)]
            );
            assert!(heard.held.is_some());
            let (_, seen) = frame(&mut touches, time + 0.12, vec![]);
            assert_eq!(seen, vec![press_with(at, PointerButton::Primary, false, Modifiers::SHIFT)], "Shift outlasts the release's frame");
            let (_, seen) = frame(&mut touches, time + 0.14, vec![]);
            assert_eq!(seen, vec![Event::PointerGone, Event::ModifiersChanged(Modifiers::NONE)]);
        }
        // The held finger lifts after Shift clicks: no right click.
        let (_, seen) = frame(&mut touches, 4.0, vec![finger(1, TouchPhase::End, down)]);
        assert_eq!(pressed(&seen), 0, "{seen:?}");
        assert_eq!(touches.mode, Mode::Idle);
    }

    #[test]
    fn a_second_fingers_tap_before_the_hold_is_a_shift_click_and_cancels_the_right_click() {
        let mut touches = Touches::default();
        let (down, there) = (at(100.0, 900.0), at(500.0, 400.0));
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.2, vec![finger(2, TouchPhase::Start, there)]);
        let (heard, seen) = frame(&mut touches, 1.3, vec![finger(2, TouchPhase::End, there)]);
        assert_eq!(
            seen,
            vec![
                wheel(TouchPhase::End, Vec2::ZERO),
                Event::ModifiersChanged(Modifiers::SHIFT),
                Event::PointerMoved(there),
                press_with(there, PointerButton::Primary, true, Modifiers::SHIFT),
            ]
        );
        assert_eq!(heard.held, Some(down), "the first is held now");
        frame(&mut touches, 1.32, vec![]);
        frame(&mut touches, 1.34, vec![]);
        // Another tap after the hold's time: Shift again.
        frame(&mut touches, 2.0, vec![finger(3, TouchPhase::Start, there)]);
        let (_, seen) = frame(&mut touches, 2.1, vec![finger(3, TouchPhase::End, there)]);
        assert!(seen.contains(&Event::ModifiersChanged(Modifiers::SHIFT)), "{seen:?}");
        frame(&mut touches, 2.12, vec![]);
        // And the first lifts with no right click.
        let (_, seen) = frame(&mut touches, 3.0, vec![finger(1, TouchPhase::End, down)]);
        assert_eq!(pressed(&seen), 0, "{seen:?}");
        // A second finger that comes down and lifts after the hold's time is no tap.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.2, vec![finger(2, TouchPhase::Start, there)]);
        let (_, seen) = frame(&mut touches, 1.2 + HOLD + 0.1, vec![finger(2, TouchPhase::End, there)]);
        assert_eq!(pressed(&seen), 0, "{seen:?}");
    }

    #[test]
    fn a_second_finger_that_moves_while_one_is_held_pans_with_it() {
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.0 + HOLD, vec![]);
        frame(&mut touches, 2.0, vec![finger(2, TouchPhase::Start, at(500.0, 400.0))]);
        let (heard, seen) = frame(&mut touches, 2.1, vec![finger(2, TouchPhase::Move, at(540.0, 400.0))]);
        assert!(heard.held.is_none());
        assert_eq!(phases(&seen).first(), Some(&TouchPhase::Start), "{seen:?}");
        assert!(matches!(touches.mode, Mode::Two { .. }));
        let (_, seen) = frame(&mut touches, 2.2, vec![finger(2, TouchPhase::Move, at(580.0, 400.0))]);
        assert!(scrolled(&seen).0.x > 0.0, "{seen:?}");
        assert_eq!(pressed(&seen), 0);
        // A held finger that slides off alone does nothing.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.0 + HOLD, vec![]);
        let (heard, seen) = frame(&mut touches, 2.0, vec![finger(1, TouchPhase::Move, at(400.0, 400.0))]);
        assert!(heard.held.is_none() && heard.drag.is_none() && pressed(&seen) == 0, "{seen:?}");
        let (_, seen) = frame(&mut touches, 2.1, vec![finger(1, TouchPhase::End, at(400.0, 400.0))]);
        assert_eq!(pressed(&seen), 0, "{seen:?}");
    }

    #[test]
    fn a_finger_that_moves_or_lifts_before_the_hold_is_no_right_click() {
        let secondary = |events: &[Event]| events.iter().any(|event| matches!(event, Event::PointerButton { button: PointerButton::Secondary, .. }));
        let down = at(300.0, 400.0);
        // Lifted just before: a click.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        let (_, seen) = frame(&mut touches, 1.0 + HOLD - 0.01, vec![finger(1, TouchPhase::End, down)]);
        assert_eq!(pressed(&seen), 1);
        assert!(!secondary(&seen));
        // Moved past the slop first: a drag, however long it lasts.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        let (heard, _) = frame(&mut touches, 1.2, vec![finger(1, TouchPhase::Move, at(330.0, 400.0))]);
        assert!(heard.drag.is_some());
        let (_, seen) = frame(&mut touches, 2.0, vec![]);
        assert!(!secondary(&seen), "{seen:?}");
        // A second finger before the hold: two fingers, no right click.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 1.2, vec![finger(2, TouchPhase::Start, at(400.0, 400.0))]);
        let (_, seen) = frame(&mut touches, 2.0, vec![]);
        assert!(!secondary(&seen), "{seen:?}");
        // The Pencil's tip before the hold: the tip's, no right click.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        let (_, seen) = frame(&mut touches, 1.2, vec![tip(2, TouchPhase::Start, at(500.0, 400.0))]);
        assert!(!secondary(&seen));
        let (_, seen) = frame(&mut touches, 2.0, vec![]);
        assert!(!secondary(&seen), "{seen:?}");
    }

    #[test]
    fn a_finger_taken_back_clicks_nothing() {
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        frame(&mut touches, 3.0, vec![finger(2, TouchPhase::Start, down)]);
        let (_, seen) = frame(&mut touches, 3.1, vec![finger(2, TouchPhase::Cancel, down)]);
        assert_eq!(seen, vec![Event::PointerGone]);
        let (_, seen) = frame(&mut touches, 3.2, vec![]);
        assert!(seen.is_empty(), "no release is owed: {seen:?}");
    }

    #[test]
    fn two_fingers_are_a_bracketed_scroll_and_a_pinch_about_the_point_between_them() {
        let mut touches = Touches::default();
        let (a, b) = (at(400.0, 500.0), at(520.0, 500.0));
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, a)]);
        let (heard, seen) = frame(&mut touches, 1.02, vec![finger(2, TouchPhase::Start, b)]);
        assert_eq!(seen, vec![Event::PointerMoved(at(460.0, 500.0)), wheel(TouchPhase::Start, Vec2::ZERO)]);
        assert!(heard.fingers && heard.drag.is_none());
        // Both move right and down, and apart to twice the distance, over ten frames.
        let mut all = Vec::new();
        for step in 1..=10 {
            let part = step as f32 / 10.0;
            let off = vec2(40.0, 20.0) * part;
            let half = 60.0 * (1.0 + part);
            let (a, b) = (at(460.0 - half, 500.0) + off, at(460.0 + half, 500.0) + off);
            let (heard, seen) =
                frame(&mut touches, 1.02 + 0.01 * f64::from(step), vec![finger(1, TouchPhase::Move, a), finger(2, TouchPhase::Move, b)]);
            assert!(heard.fingers && heard.drag.is_none());
            assert_eq!(seen.iter().rfind(|event| matches!(event, Event::PointerMoved(_))), Some(&Event::PointerMoved(between(a, b))));
            all.extend(seen);
        }
        let (moved, zoomed) = scrolled(&all);
        assert!((moved - vec2(40.0, 20.0)).length() < 1e-3, "{moved:?}");
        assert!((zoomed - 2.0).abs() < 1e-3, "{zoomed}");
        assert_eq!(pressed(&all) + released(&all), 0, "two fingers never press");
        assert!(phases(&all).iter().all(|phase| *phase == TouchPhase::Move));
        assert!(touches.readout().iter().any(|line| line.starts_with("touch: two fingers")), "{:?}", touches.readout());
        let (heard, seen) = frame(&mut touches, 1.2, vec![finger(1, TouchPhase::End, at(380.0, 520.0))]);
        assert_eq!(seen, vec![wheel(TouchPhase::End, Vec2::ZERO), Event::PointerGone]);
        assert_eq!(heard.notes.len(), 1);
        assert!(heard.notes.iter().all(|note| note.contains("moved 40, 20 pt") && note.contains("x2.00")), "{:?}", heard.notes);
    }

    #[test]
    fn a_second_finger_ends_a_drag_without_a_click_and_begins_the_scroll() {
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        let far = at(360.0, 400.0);
        frame(&mut touches, 0.4, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 0.5, vec![finger(1, TouchPhase::Move, far)]);
        frame(&mut touches, 0.51, vec![]);
        let second = at(460.0, 400.0);
        let (heard, seen) = frame(&mut touches, 1.04, vec![finger(2, TouchPhase::Start, second)]);
        assert_eq!(seen, vec![button(far, false), Event::PointerMoved(at(410.0, 400.0)), wheel(TouchPhase::Start, Vec2::ZERO)]);
        assert!(far.distance(down) > 6.0, "further than a click may move");
        assert_eq!(heard.drag, Some(down), "the release's frame is still the drag's");
        assert!(!heard.taken_back, "a drag as old as this one was meant");
        assert_eq!(heard.notes, vec!["one finger dragged 60 pt in 0.54 s, then a second came down".to_owned()]);
        let (heard, _) = frame(&mut touches, 1.05, vec![]);
        assert!(heard.drag.is_none());
        assert!(heard.fingers);
    }

    #[test]
    fn a_second_finger_in_the_first_ones_first_moments_finds_no_drag_to_end() {
        let (a, on, b) = (at(400.0, 500.0), at(420.0, 500.0), at(540.0, 500.0));
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, a)]);
        frame(&mut touches, 1.03, vec![finger(1, TouchPhase::Move, on)]);
        let (heard, seen) = frame(&mut touches, 1.05, vec![finger(2, TouchPhase::Start, b)]);
        assert_eq!(seen, vec![Event::PointerMoved(between(on, b)), wheel(TouchPhase::Start, Vec2::ZERO)]);
        assert!(heard.drag.is_none() && !heard.taken_back && heard.wake.is_none());
        assert_eq!(heard.notes, vec!["a second finger came down 0.05 s after the first, which had moved 20 pt".to_owned()]);
        assert!(touches.readout().iter().all(|line| !line.contains("second finger")), "the log's alone: {:?}", touches.readout());
        // The two then scroll from where they are, the first finger's head start not counted.
        let (_, seen) = frame(&mut touches, 1.07, vec![finger(1, TouchPhase::Move, at(430.0, 510.0)), finger(2, TouchPhase::Move, at(550.0, 510.0))]);
        assert!((scrolled(&seen).0 - vec2(10.0, 10.0)).length() < 1e-3, "{seen:?}");
        assert_eq!(pressed(&seen), 0);

        // Long after its first moments, but in one frame's input: the first finger's way past
        // the slop, then the second finger. Still no drag.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, a)]);
        let (heard, seen) = frame(&mut touches, 2.0, vec![finger(1, TouchPhase::Move, on), finger(2, TouchPhase::Start, b)]);
        assert_eq!(seen, vec![Event::PointerMoved(on), Event::PointerMoved(between(on, b)), wheel(TouchPhase::Start, Vec2::ZERO)]);
        assert!(heard.drag.is_none() && !heard.taken_back);
    }

    #[test]
    fn a_second_finger_or_the_tip_soon_after_the_first_takes_its_drag_back() {
        let (down, far, second) = (at(300.0, 400.0), at(360.0, 400.0), at(460.0, 400.0));
        // The first finger's drag has begun and has been shown to the app.
        let begun = || {
            let mut touches = Touches::default();
            frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
            let (heard, _) = frame(&mut touches, 1.1, vec![finger(1, TouchPhase::Move, far)]);
            assert_eq!(heard.drag, Some(down));
            frame(&mut touches, 1.11, vec![]);
            touches
        };
        let mut touches = begun();
        let (heard, seen) = frame(&mut touches, 1.2, vec![finger(2, TouchPhase::Start, second)]);
        assert_eq!(seen, vec![button(far, false), Event::PointerMoved(at(410.0, 400.0)), wheel(TouchPhase::Start, Vec2::ZERO)]);
        assert!(heard.taken_back);
        assert_eq!(heard.drag, Some(down), "the release's frame is still the drag's");
        assert_eq!(heard.notes, vec!["one finger dragged 60 pt in 0.10 s, then a second came down: taken back".to_owned()]);
        let (heard, _) = frame(&mut touches, 1.21, vec![]);
        assert!(!heard.taken_back && heard.drag.is_none());

        // The Pencil's tip after a hand: the same, and the frame is the tip's.
        let mut touches = begun();
        let nib = at(700.0, 200.0);
        let (heard, seen) = frame(&mut touches, 1.3, vec![tip(7, TouchPhase::Start, nib)]);
        assert_eq!(seen, vec![button(far, false), Event::PointerMoved(nib), button(nib, true)]);
        assert!(heard.taken_back && heard.drag.is_none());
        assert_eq!(heard.notes, vec!["one finger dragged 60 pt in 0.20 s, then the Pencil came down: taken back".to_owned()]);

        // Not after JOIN, counted from the drag's beginning.
        let mut touches = begun();
        let (heard, _) = frame(&mut touches, 1.1 + JOIN - 0.01, vec![finger(2, TouchPhase::Start, second)]);
        assert!(heard.taken_back);
        let mut touches = begun();
        let (heard, _) = frame(&mut touches, 1.1 + JOIN + 0.01, vec![finger(2, TouchPhase::Start, second)]);
        assert!(!heard.taken_back);
    }

    #[test]
    fn after_two_fingers_the_one_left_does_nothing_until_a_second_joins_it_again() {
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, at(400.0, 500.0)), finger(2, TouchPhase::Start, at(500.0, 500.0))]);
        // They pan (a still second finger lifted soon would be a Shift tap).
        frame(&mut touches, 1.05, vec![finger(1, TouchPhase::Move, at(400.0, 530.0)), finger(2, TouchPhase::Move, at(500.0, 530.0))]);
        frame(&mut touches, 1.1, vec![finger(2, TouchPhase::End, at(500.0, 530.0))]);
        // The one left moves a long way: no drag, no pointer.
        let (heard, seen) = frame(&mut touches, 1.2, vec![finger(1, TouchPhase::Move, at(600.0, 700.0))]);
        assert!(seen.is_empty(), "{seen:?}");
        assert!(!heard.fingers && heard.drag.is_none());
        // A second joins it: a scroll again, between where they now are.
        let (_, seen) = frame(&mut touches, 1.3, vec![finger(3, TouchPhase::Start, at(700.0, 700.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(at(650.0, 700.0)), wheel(TouchPhase::Start, Vec2::ZERO)]);
        frame(&mut touches, 1.4, vec![finger(3, TouchPhase::End, at(700.0, 700.0))]);
        let (_, seen) = frame(&mut touches, 1.5, vec![finger(1, TouchPhase::End, at(600.0, 700.0))]);
        assert!(seen.is_empty(), "the one left lifts without a click: {seen:?}");
        // And the next finger is a fresh one.
        let (_, seen) = frame(&mut touches, 1.6, vec![finger(4, TouchPhase::Start, at(100.0, 100.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(at(100.0, 100.0))]);
    }

    #[test]
    fn a_third_finger_does_nothing() {
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, at(400.0, 500.0)), finger(2, TouchPhase::Start, at(500.0, 500.0))]);
        let (_, seen) = frame(
            &mut touches,
            1.1,
            vec![
                finger(3, TouchPhase::Start, at(900.0, 900.0)),
                finger(3, TouchPhase::Move, at(950.0, 990.0)),
                finger(3, TouchPhase::End, at(950.0, 990.0)),
            ],
        );
        assert!(seen.is_empty(), "{seen:?}");
        // The two go on.
        let (_, seen) = frame(&mut touches, 1.2, vec![finger(1, TouchPhase::Move, at(400.0, 520.0)), finger(2, TouchPhase::Move, at(500.0, 520.0))]);
        assert!((scrolled(&seen).0 - vec2(0.0, 20.0)).length() < 1e-3, "{seen:?}");
    }

    #[test]
    fn the_pencils_tip_is_the_left_button_at_once() {
        let mut touches = Touches::default();
        let (down, on, up) = (at(200.0, 300.0), at(203.0, 301.0), at(260.0, 340.0));
        let (heard, seen) = frame(&mut touches, 1.0, vec![tip(7, TouchPhase::Start, down)]);
        assert_eq!(seen, vec![Event::PointerMoved(down), button(down, true)]);
        assert!(!heard.fingers && heard.drag.is_none() && heard.wake.is_none());
        // Within the dead zone the pointer stays where the tip came down.
        let (heard, seen) = frame(&mut touches, 1.01, vec![tip(7, TouchPhase::Move, on)]);
        assert!(seen.is_empty(), "{seen:?}");
        assert!(!heard.fingers);
        assert_eq!(touches.readout(), vec!["touch: the Pencil's tip".to_owned()]);
        let (heard, seen) = frame(&mut touches, 1.3, vec![tip(7, TouchPhase::End, up)]);
        assert_eq!(seen, vec![Event::PointerMoved(up), button(up, false)]);
        assert!(heard.drag.is_none());
        assert_eq!(heard.wake, Some(0.0));
        assert_eq!(heard.notes, vec!["the Pencil's tip lifted at 260, 340".to_owned()]);
        assert!(touches.readout().is_empty(), "the log's alone: {:?}", touches.readout());
        // The tip hovers on in the next frame: the pointer is the hover's, not gone after it.
        let hover = at(262.0, 338.0);
        let mut raw = egui::RawInput { time: Some(1.31), events: vec![Event::PointerMoved(hover)], ..Default::default() };
        touches.frame(&mut raw);
        assert_eq!(raw.events, vec![Event::PointerGone, Event::PointerMoved(hover)]);
        let (_, seen) = frame(&mut touches, 1.32, vec![]);
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn a_pencil_tip_that_slides_or_rests_within_its_dead_zone_moves_nothing_and_taps_where_it_came_down() {
        let mut touches = Touches::default();
        let down = at(200.0, 300.0);
        frame(&mut touches, 1.0, vec![tip(7, TouchPhase::Start, down)]);
        // It slides most of the dead zone and rests there far past egui's click time.
        let slid = down + vec2(PENCIL_SLOP - 1.0, 0.0);
        let (_, seen) = frame(&mut touches, 1.05, vec![tip(7, TouchPhase::Move, slid)]);
        assert!(seen.is_empty(), "{seen:?}");
        let (_, seen) = frame(&mut touches, 3.0, vec![tip(7, TouchPhase::Move, slid + vec2(0.0, 0.5))]);
        assert!(seen.is_empty(), "a resting tip's tremor moves nothing: {seen:?}");
        let (_, seen) = frame(&mut touches, 3.1, vec![tip(7, TouchPhase::End, slid)]);
        assert_eq!(seen, vec![Event::PointerMoved(down), button(down, false)], "let go where it came down");
    }

    #[test]
    fn a_pencil_tip_past_its_dead_zone_drags_from_the_tip_on() {
        let mut touches = Touches::default();
        let down = at(200.0, 300.0);
        frame(&mut touches, 1.0, vec![tip(7, TouchPhase::Start, down)]);
        let past = down + vec2(0.0, PENCIL_SLOP + 1.0);
        let (_, seen) = frame(&mut touches, 1.05, vec![tip(7, TouchPhase::Move, past)]);
        assert_eq!(seen, vec![Event::PointerMoved(past)], "it catches up at once");
        // Back inside the circle it still follows: the dead zone is only the start's.
        let (_, seen) = frame(&mut touches, 1.1, vec![tip(7, TouchPhase::Move, down)]);
        assert_eq!(seen, vec![Event::PointerMoved(down)]);
        let (_, seen) = frame(&mut touches, 1.2, vec![tip(7, TouchPhase::End, down + vec2(2.0, 0.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(down + vec2(2.0, 0.0)), button(down + vec2(2.0, 0.0), false)]);
    }

    #[test]
    fn fingers_do_nothing_while_the_tip_is_down_and_stay_dead_until_they_lift() {
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![tip(7, TouchPhase::Start, at(200.0, 300.0))]);
        // A hand comes to rest and slides.
        let (heard, seen) =
            frame(&mut touches, 1.1, vec![finger(1, TouchPhase::Start, at(600.0, 800.0)), finger(1, TouchPhase::Move, at(660.0, 860.0))]);
        assert!(seen.is_empty(), "{seen:?}");
        assert!(!heard.fingers);
        frame(&mut touches, 1.2, vec![tip(7, TouchPhase::End, at(200.0, 300.0))]);
        // The tip is up, the hand still down: it stays dead, a tap of it included. (The tip's
        // pointer goes, a frame after its release.)
        let (_, seen) = frame(&mut touches, 1.3, vec![finger(1, TouchPhase::Move, at(700.0, 900.0)), finger(1, TouchPhase::End, at(700.0, 900.0))]);
        assert_eq!(seen, vec![Event::PointerGone]);
        // The next finger counts again.
        let (_, seen) = frame(&mut touches, 1.4, vec![finger(2, TouchPhase::Start, at(100.0, 100.0))]);
        assert_eq!(seen, vec![Event::PointerMoved(at(100.0, 100.0))]);
    }

    #[test]
    fn the_tip_coming_down_ends_what_the_fingers_were_doing() {
        // A drag: let go where it was, and the tip presses in the same frame, which is no longer
        // the drag's.
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        let far = at(380.0, 400.0);
        frame(&mut touches, 0.4, vec![finger(1, TouchPhase::Start, down)]);
        frame(&mut touches, 0.5, vec![finger(1, TouchPhase::Move, far)]);
        frame(&mut touches, 0.51, vec![]);
        let nib = at(700.0, 200.0);
        let (heard, seen) = frame(&mut touches, 1.04, vec![tip(7, TouchPhase::Start, nib)]);
        assert_eq!(seen, vec![button(far, false), Event::PointerMoved(nib), button(nib, true)]);
        assert!(heard.drag.is_none() && !heard.fingers && !heard.taken_back);
        let (_, seen) = frame(&mut touches, 1.05, vec![finger(1, TouchPhase::Move, at(500.0, 500.0)), finger(1, TouchPhase::End, at(500.0, 500.0))]);
        assert!(seen.is_empty(), "the finger is dead: {seen:?}");

        // Two fingers: the scroll ends, then the tip presses.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, at(400.0, 500.0)), finger(2, TouchPhase::Start, at(500.0, 500.0))]);
        let (_, seen) = frame(&mut touches, 1.1, vec![tip(7, TouchPhase::Start, nib)]);
        assert_eq!(seen, vec![wheel(TouchPhase::End, Vec2::ZERO), Event::PointerMoved(nib), button(nib, true)]);

        // A finger not yet moved: no click of its own, now or when it lifts.
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        let (_, seen) = frame(&mut touches, 1.1, vec![tip(7, TouchPhase::Start, nib), finger(1, TouchPhase::End, down)]);
        assert_eq!(seen, vec![Event::PointerMoved(nib), button(nib, true)]);
    }

    #[test]
    fn a_tap_owed_its_release_gets_it_before_the_next_press() {
        let mut touches = Touches::default();
        let down = at(300.0, 400.0);
        let nib = at(700.0, 200.0);
        // The tap's lift and the tip's landing in one frame.
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, down)]);
        let (_, seen) = frame(&mut touches, 1.1, vec![finger(1, TouchPhase::End, down), tip(7, TouchPhase::Start, nib)]);
        assert_eq!(seen, vec![Event::PointerMoved(down), button(down, true), button(down, false), Event::PointerMoved(nib), button(nib, true)]);
        let (_, seen) = frame(&mut touches, 1.2, vec![]);
        assert!(seen.is_empty(), "nothing is owed twice, and the tip's pointer stays: {seen:?}");
    }

    #[test]
    fn the_window_layers_pointer_for_a_touch_goes_and_any_other_stays() {
        let mut touches = Touches::default();
        let hover = at(50.0, 60.0);
        let down = at(300.0, 400.0);
        let key = Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        // A hover of the Pencil, then a finger as the window layer reports it (the touch, then
        // the pointer it makes of it), then a key, then a hover again.
        let mut raw = egui::RawInput {
            time: Some(1.0),
            events: vec![
                Event::PointerMoved(hover),
                finger(1, TouchPhase::Start, down),
                Event::PointerMoved(down),
                button(down, true),
                key.clone(),
                Event::PointerMoved(hover),
            ],
            ..Default::default()
        };
        touches.frame(&mut raw);
        assert_eq!(
            raw.events,
            vec![Event::PointerMoved(hover), finger(1, TouchPhase::Start, down), Event::PointerMoved(down), key, Event::PointerMoved(hover)],
            "the window layer's press is gone, the touch itself and the hovers are not"
        );
        // Its release and its going, likewise.
        let mut raw = egui::RawInput {
            time: Some(3.0),
            events: vec![finger(1, TouchPhase::End, down), button(down, false), Event::PointerGone],
            ..Default::default()
        };
        touches.frame(&mut raw);
        assert_eq!(raw.events, vec![finger(1, TouchPhase::End, down), Event::PointerGone], "held too long for a tap: no click at all");
    }

    #[test]
    fn fingers_too_close_together_do_not_zoom() {
        let mut touches = Touches::default();
        frame(&mut touches, 1.0, vec![finger(1, TouchPhase::Start, at(400.0, 500.0)), finger(2, TouchPhase::Start, at(410.0, 500.0))]);
        let (_, seen) = frame(&mut touches, 1.1, vec![finger(2, TouchPhase::Move, at(420.0, 500.0))]);
        let (moved, zoomed) = scrolled(&seen);
        assert_eq!(zoomed, 1.0, "{seen:?}");
        assert!((moved - vec2(5.0, 0.0)).length() < 1e-3, "it still scrolls: {seen:?}");
    }

    #[test]
    fn hostile_input_never_panics_and_never_leaves_the_button_down() {
        let mut touches = Touches::default();
        let nowhere = Pos2::new(f32::NAN, f32::INFINITY);
        let edge = Pos2::new(f32::MAX, -f32::MAX);
        let mut down = 0_i32;
        let mut time = 0.0;
        for round in 0..200_u64 {
            let events = vec![
                finger(round % 5, TouchPhase::Start, if round % 7 == 0 { nowhere } else { at(round as f32, 50.0) }),
                finger(round % 3, TouchPhase::Move, if round % 4 == 0 { edge } else { at(900.0 - round as f32, 300.0) }),
                tip(
                    100 + round % 2,
                    if round % 6 == 0 { TouchPhase::Start } else { TouchPhase::Move },
                    if round % 9 == 0 { nowhere } else { at(10.0, 10.0) },
                ),
                finger(round % 4, if round % 2 == 0 { TouchPhase::End } else { TouchPhase::Cancel }, nowhere),
                tip(100 + round % 2, if round % 5 == 0 { TouchPhase::End } else { TouchPhase::Move }, edge),
                finger(1000 + round, TouchPhase::Start, at(5.0, 5.0)),
            ];
            let mut raw = egui::RawInput {
                time: [Some(time), None, Some(f64::NAN), Some(-1.0)].get(round as usize % 4).copied().flatten(),
                events,
                ..Default::default()
            };
            let heard = touches.frame(&mut raw);
            assert!(heard.wake.is_none_or(|wake| (0.0..=SETTLE.max(HOLD)).contains(&wake)));
            for event in &raw.events {
                match event {
                    Event::PointerButton { pressed: true, .. } => down += 1,
                    Event::PointerButton { pressed: false, .. } => down -= 1,
                    Event::Zoom(factor) => assert!(factor.is_finite() && *factor > 0.0),
                    Event::MouseWheel { delta, .. } => assert!(delta.is_finite()),
                    Event::PointerMoved(at) => assert!(at.is_finite()),
                    _ => {}
                }
                assert!((0..=1).contains(&down), "round {round}: the button went down twice or up unpressed");
            }
            assert!(touches.ignored.len() <= MAX_IGNORED);
            let _ = touches.readout();
            time += 0.01;
        }
    }

    #[test]
    fn the_self_tests_fingers_tap_then_pan_then_pinch_then_drag() {
        let view = Rect::from_min_max(at(0.0, 80.0), at(1032.0, 1200.0));
        let mut touches = Touches::default();
        let mut parts: [Vec<Event>; 4] = Default::default();
        let mut drags = [false; 4];
        let mut step = 0;
        while let Some(events) = demo(step, view) {
            assert!(events.iter().all(|event| matches!(event, Event::Touch { pos, .. } if view.contains(*pos))), "step {step}");
            let (heard, seen) = frame(&mut touches, f64::from(step) / 60.0, events);
            let part = [DEMO_TAP_DONE, DEMO_PAN_DONE, DEMO_PINCH_DONE].iter().filter(|done| step > **done).count();
            drags[part] |= heard.drag.is_some();
            parts[part].extend(seen);
            step += 1;
        }
        assert_eq!(step, DEMO_DRAG_DONE + 1);
        let [tap, pan, pinch, drag] = parts;
        // A click at the view's centre, and nothing else.
        let clicks = |events: &[Event]| -> Vec<(Pos2, bool)> {
            events
                .iter()
                .filter_map(|event| if let Event::PointerButton { pos, pressed, .. } = event { Some((*pos, *pressed)) } else { None })
                .collect()
        };
        assert_eq!(clicks(&tap), vec![(view.center(), true), (view.center(), false)]);
        assert!(phases(&tap).is_empty() && scrolled(&tap).1 == 1.0);
        // Carried, not zoomed.
        let (moved, zoomed) = scrolled(&pan);
        assert!((moved - DEMO_PAN).length() < 1e-3 && (zoomed - 1.0).abs() < 1e-4, "{moved:?} {zoomed}");
        assert_eq!(phases(&pan).first(), Some(&TouchPhase::Start));
        assert_eq!(phases(&pan).last(), Some(&TouchPhase::End));
        // Zoomed about a point that stays put.
        let (moved, zoomed) = scrolled(&pinch);
        assert!(moved.length() < 1e-3 && (zoomed - DEMO_PINCH).abs() < 1e-3, "{moved:?} {zoomed}");
        // Dragged with the button, and with nothing else.
        assert_eq!((pressed(&pan), pressed(&pinch), pressed(&drag)), (0, 0, 1));
        assert_eq!(drags, [false, false, false, true]);
        assert!(phases(&drag).is_empty() && scrolled(&drag).1 == 1.0);
        let from = view.center() + vec2(0.0, 40.0);
        assert_eq!(clicks(&drag), vec![(from, true), (from + vec2(DEMO_DRAG, 0.0), false)]);
        // The app is shown the whole way: its pointer ends where the finger lifted.
        assert_eq!(drag.iter().rfind(|event| matches!(event, Event::PointerMoved(_))), Some(&Event::PointerMoved(from + vec2(DEMO_DRAG, 0.0))));
        // All fingers are up at the end.
        assert_eq!(touches.mode, Mode::Idle);
        assert!(touches.pencil.is_none() && touches.ignored.is_empty() && touches.release.is_none() && !touches.gone);
        assert_eq!(drag.last(), Some(&Event::PointerGone));
    }
}
