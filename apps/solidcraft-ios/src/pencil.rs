//! What the Apple Pencil's own gestures do in the iPad spike.
//!
//! The window layer (`vendor/winit`, iOS) reports a hovering Pencil as the pointer, and the
//! Pencil's own gestures as function keys that no iPad keyboard sends through it: F13 pressed is
//! a double tap on the Pencil's side, F14 is down for as long as a squeeze lasts, and F15 pressed
//! says the system took the squeeze back. [`Pencil::frame`] takes those keys out of a frame's
//! input and answers with what the app should do instead:
//!
//! - a double tap is Escape;
//! - a squeeze let go before [`HOLD`] is a click where the tip hovered [`LOOK_BACK`] before the
//!   squeeze was felt, because the squeezing fingers move the tip and the hand had settled on the
//!   point before that. Until the click is delivered the app sees the pointer stay on that point;
//! - a squeeze held for [`HOLD`] opens a ring of keys around that point, for what a keyboard does
//!   on the desktop. Letting go over a key presses it, as a tap on it does. Shift and Ctrl stay
//!   down for the next click or key only, since nothing can hold them down here.
//!
//! Nothing here is specific to iOS: it reads and writes egui's input only, so it is tested on any
//! platform.

use std::collections::VecDeque;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};

/// A squeeze let go sooner than this (seconds) is a click; one held this long opens the ring.
pub const HOLD: f64 = 0.35;
/// A squeeze clicks where the tip hovered this long (seconds) before the squeeze was felt.
pub const LOOK_BACK: f64 = 0.08;
/// Hover positions older than this (seconds) are forgotten, bar the newest of them.
const REMEMBER: f64 = 1.0;
/// From the ring's centre to the centre of each of its keys, in points.
pub const RING_RADIUS: f32 = 92.0;
/// The side of a key of the ring, in points.
pub const KEY_SIDE: f32 = 56.0;
/// The ring's keys keep this far from the screen's edges, in points.
const RING_MARGIN: f32 = 8.0;
/// The readout keeps this many of the latest gestures.
const NOTES: usize = 6;

const DOUBLE_TAP: Key = Key::F13;
const SQUEEZE: Key = Key::F14;
const SQUEEZE_TAKEN: Key = Key::F15;

/// A key of the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Enter,
    Tab,
    Delete,
    Redo,
    Escape,
    Undo,
    Ctrl,
    Shift,
}

/// The ring's keys, clockwise from the top.
pub const ITEMS: [Item; 8] = [Item::Enter, Item::Tab, Item::Delete, Item::Redo, Item::Escape, Item::Undo, Item::Ctrl, Item::Shift];

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };

impl Item {
    /// The key's name, as on a keyboard.
    pub fn label(self) -> &'static str {
        match self {
            Self::Enter => "Enter",
            Self::Tab => "Tab",
            Self::Delete => "Delete",
            Self::Redo => "Redo",
            Self::Escape => "Esc",
            Self::Undo => "Undo",
            Self::Ctrl => "Ctrl",
            Self::Shift => "Shift",
        }
    }

    /// The modifier this key holds down for the next click or key, if it is one.
    pub fn holds(self) -> Option<Modifiers> {
        match self {
            Self::Ctrl => Some(CTRL),
            Self::Shift => Some(Modifiers::SHIFT),
            _ => None,
        }
    }

    /// The key (and its modifiers) this one presses, if it holds nothing.
    fn presses(self) -> Option<(Key, Modifiers)> {
        match self {
            Self::Enter => Some((Key::Enter, Modifiers::NONE)),
            Self::Tab => Some((Key::Tab, Modifiers::NONE)),
            Self::Delete => Some((Key::Delete, Modifiers::NONE)),
            Self::Redo => Some((Key::Y, CTRL)),
            Self::Escape => Some((Key::Escape, Modifiers::NONE)),
            Self::Undo => Some((Key::Z, CTRL)),
            Self::Ctrl | Self::Shift => None,
        }
    }
}

/// What a gesture asks of the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Press and release the primary button at a point.
    Click { at: Pos2, modifiers: Modifiers },
    /// Press and release a key.
    Key { key: Key, modifiers: Modifiers },
}

impl Action {
    /// The action as input events, in the shape the app's own synthetic input takes
    /// (`SolveApp::synthetic`): the modifiers, the press, the release, the modifiers let go.
    pub fn events(self) -> Vec<Event> {
        match self {
            Self::Click { at, modifiers } => vec![
                Event::ModifiersChanged(modifiers),
                Event::PointerMoved(at),
                Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers },
                Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers },
                Event::ModifiersChanged(Modifiers::NONE),
            ],
            Self::Key { key, modifiers } => vec![
                Event::ModifiersChanged(modifiers),
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
                Event::ModifiersChanged(Modifiers::NONE),
            ],
        }
    }
}

/// What [`Pencil::frame`] heard in a frame's input.
#[derive(Debug, Default)]
pub struct Heard {
    /// What to do, in order.
    pub actions: Vec<Action>,
    /// Look again after this many seconds even if no input arrives: a squeeze still down may
    /// become a hold, and a modifier let go must be seen to be.
    pub wake: Option<f64>,
}

/// A squeeze that has not been let go.
#[derive(Debug)]
struct Squeeze {
    began: f64,
    /// Where it clicks: the tip's place before the squeeze.
    at: Option<Pos2>,
    /// It lasted [`HOLD`] and opened the ring.
    held: bool,
    /// The furthest the tip went from `at` while squeezed, in points.
    drift: f32,
}

/// The Pencil's gestures, from one frame to the next.
#[derive(Debug, Default)]
pub struct Pencil {
    /// Where the tip hovered lately (seconds, point), oldest first. Empty out of range.
    hover: VecDeque<(f64, Pos2)>,
    /// The primary button is down: the tip (or a finger) is on the screen.
    down: bool,
    squeeze: Option<Squeeze>,
    /// Where the app sees the pointer while a squeeze and its click last, whatever the tip does.
    pin: Option<Pos2>,
    /// A click of ours is on its way through the app's synthetic input.
    clicking: bool,
    /// The tip went out of range while pinned; the app is told once the pin lifts.
    gone_while_pinned: bool,
    /// The centre of the ring of keys, while it is open.
    ring: Option<Pos2>,
    /// The press now down began on the ring.
    on_ring: bool,
    /// The modifiers held down for the next click or key.
    held: Modifiers,
    /// The click the modifiers were held for has been made: let go at the next frame.
    let_go: bool,
    /// The modifiers egui was last told are down.
    told: Modifiers,
    notes: VecDeque<String>,
    now: f64,
}

impl Pencil {
    /// Read one frame's input before the app does: take the Pencil's gestures out, keep the
    /// pointer where a squeeze clicks, add the held modifiers to clicks and keys. `screen` is
    /// where the ring may be; `busy` says the app still has synthetic input to deliver.
    pub fn frame(&mut self, raw: &mut egui::RawInput, screen: Rect, busy: bool) -> Heard {
        let now = raw.time.filter(|t| t.is_finite()).unwrap_or(self.now);
        self.now = now;
        let mut heard = Heard::default();
        let mut events = Vec::with_capacity(raw.events.len() + 2);

        if self.let_go {
            self.let_go = false;
            self.held = Modifiers::NONE;
        }
        // egui keeps the modifiers from frame to frame; while ours are held, say so every frame
        // (the app's own synthetic input says otherwise in passing), and once when they are not.
        if !self.held.is_none() || self.told != self.held {
            events.push(Event::ModifiersChanged(self.held));
            self.told = self.held;
        }
        if self.clicking && !busy {
            self.clicking = false;
            if self.squeeze.is_none() {
                self.unpin(&mut events);
            }
        }

        for event in std::mem::take(&mut raw.events) {
            match event {
                Event::Key { key: DOUBLE_TAP, pressed, .. } => {
                    if pressed {
                        self.double_tap(&mut heard);
                    }
                }
                Event::Key { key: SQUEEZE, pressed: true, .. } => self.squeeze_began(now),
                Event::Key { key: SQUEEZE, pressed: false, .. } => self.squeeze_ended(now, &mut heard, &mut events),
                Event::Key { key: SQUEEZE_TAKEN, pressed, .. } => {
                    if pressed && self.squeeze.take().is_some() {
                        self.note("squeeze taken back by the system: nothing done".to_owned());
                        if !self.clicking {
                            self.unpin(&mut events);
                        }
                    }
                }
                Event::PointerMoved(at) => {
                    if self.down {
                        // In contact: a stroke, never pinned.
                        events.push(Event::PointerMoved(at));
                        continue;
                    }
                    self.remember(now, at);
                    if let Some(squeeze) = &mut self.squeeze
                        && let Some(from) = squeeze.at
                    {
                        squeeze.drift = squeeze.drift.max(from.distance(at));
                    }
                    events.push(Event::PointerMoved(self.pin.unwrap_or(at)));
                }
                Event::PointerGone => {
                    self.hover.clear();
                    if self.pin.is_some() {
                        self.gone_while_pinned = true;
                    } else {
                        events.push(Event::PointerGone);
                    }
                }
                Event::PointerButton { pos, button, pressed, modifiers } => {
                    if button == PointerButton::Primary {
                        self.down = pressed;
                    }
                    if pressed {
                        self.on_ring = self.ring.is_some_and(|centre| ring_bounds(centre).contains(pos));
                        if !self.on_ring {
                            // A press elsewhere closes the ring, as it closes a menu.
                            self.ring = None;
                        }
                    } else {
                        if !self.on_ring && !self.held.is_none() {
                            self.let_go = true;
                            heard.wake = Some(0.0);
                        }
                        self.on_ring = false;
                    }
                    events.push(Event::PointerButton { pos, button, pressed, modifiers: modifiers | self.held });
                }
                Event::Key { key, physical_key, pressed, repeat, modifiers } => {
                    events.push(Event::Key { key, physical_key, pressed, repeat, modifiers: modifiers | self.held });
                }
                other => events.push(other),
            }
        }

        let waited = self.squeeze.as_ref().filter(|squeeze| !squeeze.held).map(|squeeze| now - squeeze.began);
        if let Some(waited) = waited {
            if waited >= HOLD {
                self.open_ring(screen, &mut events);
            } else {
                // A clock that ran backwards must not put the wake far off.
                let left = (HOLD - waited).clamp(0.0, HOLD);
                heard.wake = Some(heard.wake.map_or(left, |wake| wake.min(left)));
            }
        }
        raw.events = events;
        heard
    }

    /// Press a key of the ring: by letting a held squeeze go over it, or by a tap on it. The ring
    /// closes either way.
    pub fn press(&mut self, item: Item) -> Option<Action> {
        self.ring = None;
        match item.presses() {
            Some((key, modifiers)) => {
                let modifiers = modifiers | self.held;
                self.held = Modifiers::NONE;
                self.note(format!("ring: {}", item.label()));
                Some(Action::Key { key, modifiers })
            }
            None => {
                match item {
                    Item::Shift => self.held.shift = !self.held.shift,
                    _ => {
                        let on = !self.held.ctrl;
                        self.held.ctrl = on;
                        self.held.command = on;
                    }
                }
                let on = item.holds().is_some_and(|modifier| self.held.contains(modifier));
                self.note(format!("ring: {} {}", item.label(), if on { "held for the next click or key" } else { "let go" }));
                None
            }
        }
    }

    /// The centre of the ring of keys, while it is open.
    pub fn ring(&self) -> Option<Pos2> {
        self.ring
    }

    /// The modifiers held down for the next click or key.
    pub fn held(&self) -> Modifiers {
        self.held
    }

    /// Where the tip hovers, if it is in range and not on the screen.
    pub fn tip(&self) -> Option<Pos2> {
        if self.down { None } else { self.hover.back().map(|(_, at)| *at) }
    }

    /// What the Pencil is doing and the latest gestures, a line each, for the spike's readout.
    pub fn readout(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.notes.len() + 2);
        lines.push(match (self.down, self.hover.back()) {
            (true, _) => "tip: on the screen".to_owned(),
            (false, Some((_, at))) => format!("tip: hovering at {:.0}, {:.0}", at.x, at.y),
            (false, None) => "tip: out of range".to_owned(),
        });
        if !self.held.is_none() {
            lines.push(format!("held for the next click or key: {}", held_label(self.held)));
        }
        lines.extend(self.notes.iter().cloned());
        lines
    }

    fn note(&mut self, text: String) {
        if self.notes.len() >= NOTES {
            self.notes.pop_front();
        }
        self.notes.push_back(text);
    }

    fn remember(&mut self, now: f64, at: Pos2) {
        if !at.x.is_finite() || !at.y.is_finite() {
            return;
        }
        // Keep the newest position from before the horizon: it is where the tip still is.
        while self.hover.get(1).is_some_and(|(time, _)| *time <= now - REMEMBER) {
            self.hover.pop_front();
        }
        self.hover.push_back((now, at));
    }

    /// Where the tip hovered at `time`: the newest position not after it, else the oldest known.
    fn hovered_at(&self, time: f64) -> Option<Pos2> {
        self.hover.iter().rev().find(|(then, _)| *then <= time).or_else(|| self.hover.front()).map(|(_, at)| *at)
    }

    fn double_tap(&mut self, heard: &mut Heard) {
        // One layer at a time, as Escape does: the ring, then held modifiers, then the app.
        if self.ring.take().is_some() {
            self.note("double tap: ring closed".to_owned());
        } else if !self.held.is_none() {
            self.held = Modifiers::NONE;
            self.note("double tap: modifiers let go".to_owned());
        } else {
            heard.actions.push(Action::Key { key: Key::Escape, modifiers: Modifiers::NONE });
            self.note("double tap: Esc".to_owned());
        }
    }

    fn squeeze_began(&mut self, now: f64) {
        let at = if self.down { None } else { self.hovered_at(now - LOOK_BACK) };
        self.squeeze = Some(Squeeze { began: now, at, held: false, drift: 0.0 });
        self.pin = at;
        self.gone_while_pinned = false;
    }

    fn squeeze_ended(&mut self, now: f64, heard: &mut Heard, events: &mut Vec<Event>) {
        let Some(squeeze) = self.squeeze.take() else {
            return;
        };
        let lasted = now - squeeze.began;
        if squeeze.held {
            // The ring is open and the pointer is the tip's: letting go over a key presses it,
            // anywhere else leaves the ring open for a tap.
            let over = self.ring.zip(self.tip()).and_then(|(centre, tip)| item_at(centre, tip));
            if let Some(item) = over
                && let Some(action) = self.press(item)
            {
                heard.actions.push(action);
            }
            return;
        }
        let Some(at) = squeeze.at else {
            self.note(format!("squeeze {lasted:.2} s: no hover before it, nothing to click"));
            self.unpin(events);
            return;
        };
        if let Some(centre) = self.ring {
            // The ring was left open: a squeeze over a key presses it, elsewhere closes the ring.
            if let Some(item) = item_at(centre, at) {
                if let Some(action) = self.press(item) {
                    heard.actions.push(action);
                }
                self.unpin(events);
                return;
            }
            self.ring = None;
        }
        heard.actions.push(Action::Click { at, modifiers: self.held });
        self.note(format!("squeeze {lasted:.2} s: click at {:.0}, {:.0} (tip moved {:.1} pt meanwhile)", at.x, at.y, squeeze.drift));
        self.held = Modifiers::NONE;
        self.clicking = true;
    }

    fn open_ring(&mut self, screen: Rect, events: &mut Vec<Event>) {
        let at = match &mut self.squeeze {
            Some(squeeze) => {
                squeeze.held = true;
                squeeze.at
            }
            None => None,
        };
        let at = at.or_else(|| self.tip()).unwrap_or_else(|| screen.center());
        self.ring = Some(ring_centre(at, screen));
        self.note("squeeze held: ring of keys".to_owned());
        // The pointer is the tip's again, to reach the keys.
        self.unpin(events);
    }

    /// Let the app see the tip again where it is.
    fn unpin(&mut self, events: &mut Vec<Event>) {
        if self.pin.take().is_some() {
            if let Some(tip) = self.tip() {
                events.push(Event::PointerMoved(tip));
            } else if self.gone_while_pinned {
                events.push(Event::PointerGone);
            }
        }
        self.gone_while_pinned = false;
    }
}

/// The modifiers held for the next click or key, by name ("Ctrl + Shift"); empty when none are.
pub fn held_label(held: Modifiers) -> String {
    let mut names = Vec::with_capacity(2);
    if held.ctrl || held.command {
        names.push(Item::Ctrl.label());
    }
    if held.shift {
        names.push(Item::Shift.label());
    }
    names.join(" + ")
}

/// `value` kept between `low` and `high`; between the two when they cross (a screen too small).
fn within(value: f32, low: f32, high: f32) -> f32 {
    if low <= high { if value.is_finite() { value.max(low).min(high) } else { low } } else { (low + high) / 2.0 }
}

/// Where the ring's centre goes for a squeeze at `at`: there, or as near as keeps every key on
/// the screen.
fn ring_centre(at: Pos2, screen: Rect) -> Pos2 {
    let reach = RING_RADIUS + KEY_SIDE / 2.0 + RING_MARGIN;
    let centre = Pos2::new(within(at.x, screen.left() + reach, screen.right() - reach), within(at.y, screen.top() + reach, screen.bottom() - reach));
    if centre.x.is_finite() && centre.y.is_finite() { centre } else { Pos2::new(reach, reach) }
}

/// The square a key of the ring takes (`index` into [`ITEMS`], clockwise from the top).
pub fn key_rect(centre: Pos2, index: usize) -> Rect {
    let angle = std::f32::consts::TAU * (index % ITEMS.len()) as f32 / ITEMS.len() as f32 - std::f32::consts::FRAC_PI_2;
    Rect::from_center_size(centre + RING_RADIUS * egui::vec2(angle.cos(), angle.sin()), egui::Vec2::splat(KEY_SIDE))
}

/// The square around the whole ring, its dead centre included.
pub fn ring_bounds(centre: Pos2) -> Rect {
    Rect::from_center_size(centre, egui::Vec2::splat(2.0 * RING_RADIUS + KEY_SIDE))
}

fn item_at(centre: Pos2, at: Pos2) -> Option<Item> {
    ITEMS.iter().enumerate().find(|(index, _)| key_rect(centre, *index).contains(at)).map(|(_, item)| *item)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { min: Pos2::new(0.0, 24.0), max: Pos2::new(1032.0, 1356.0) };

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    /// One frame at `time` with `events`; what was heard and what the app would see.
    fn frame(pencil: &mut Pencil, time: f64, events: Vec<Event>, busy: bool) -> (Heard, Vec<Event>) {
        let mut raw = egui::RawInput { time: Some(time), events, ..Default::default() };
        let heard = pencil.frame(&mut raw, SCREEN, busy);
        (heard, raw.events)
    }

    /// The tip hovers at `at` from time 0 to `until`, a frame every 10 ms.
    fn hover(pencil: &mut Pencil, at: Pos2, until: f64) {
        let mut time = 0.0;
        while time <= until {
            let (heard, seen) = frame(pencil, time, vec![Event::PointerMoved(at)], false);
            assert!(heard.actions.is_empty());
            assert_eq!(seen.last(), Some(&Event::PointerMoved(at)));
            time += 0.01;
        }
    }

    #[test]
    fn a_double_tap_is_escape_and_its_key_never_reaches_the_app() {
        let mut pencil = Pencil::default();
        let (heard, seen) = frame(&mut pencil, 1.0, vec![key(DOUBLE_TAP, true), key(DOUBLE_TAP, false)], false);
        assert_eq!(heard.actions, vec![Action::Key { key: Key::Escape, modifiers: Modifiers::NONE }]);
        assert!(seen.is_empty(), "{seen:?}");
        assert!(heard.wake.is_none());
    }

    #[test]
    fn a_short_squeeze_clicks_where_the_tip_was_before_it_and_holds_the_pointer_there() {
        let mut pencil = Pencil::default();
        let settled = Pos2::new(400.0, 500.0);
        let nudged = Pos2::new(404.0, 507.0);
        hover(&mut pencil, settled, 0.5);
        // The squeezing fingers move the tip 30 ms before the squeeze is felt, and on through it.
        let (_, seen) = frame(&mut pencil, 0.52, vec![Event::PointerMoved(nudged)], false);
        assert_eq!(seen, vec![Event::PointerMoved(nudged)]);
        let (heard, seen) = frame(&mut pencil, 0.55, vec![Event::PointerMoved(nudged), key(SQUEEZE, true)], false);
        assert!(heard.actions.is_empty());
        assert_eq!(seen, vec![Event::PointerMoved(nudged)], "the squeeze is not known yet when this move is read");
        assert!(heard.wake.is_some_and(|wake| (wake - HOLD).abs() < 1e-9), "{:?}", heard.wake);
        // While squeezed the app sees the pointer on the settled point.
        let (heard, seen) = frame(&mut pencil, 0.60, vec![Event::PointerMoved(Pos2::new(406.0, 510.0))], false);
        assert!(heard.actions.is_empty());
        assert_eq!(seen, vec![Event::PointerMoved(settled)]);
        // Let go after 150 ms: a click on the settled point.
        let (heard, seen) = frame(&mut pencil, 0.70, vec![key(SQUEEZE, false)], false);
        assert_eq!(heard.actions, vec![Action::Click { at: settled, modifiers: Modifiers::NONE }]);
        assert!(seen.is_empty(), "{seen:?}");
        assert!(pencil.ring().is_none());
        // Until the click is delivered the pointer stays put; after it, it is the tip's again.
        let (_, seen) = frame(&mut pencil, 0.71, vec![Event::PointerMoved(nudged)], true);
        assert_eq!(seen, vec![Event::PointerMoved(settled)]);
        let (_, seen) = frame(&mut pencil, 0.75, vec![], false);
        assert_eq!(seen, vec![Event::PointerMoved(nudged)]);
        let (_, seen) = frame(&mut pencil, 0.76, vec![Event::PointerMoved(Pos2::new(300.0, 300.0))], false);
        assert_eq!(seen, vec![Event::PointerMoved(Pos2::new(300.0, 300.0))]);
        assert!(pencil.readout().iter().any(|line| line.contains("click at 400, 500")), "{:?}", pencil.readout());
    }

    #[test]
    fn a_held_squeeze_opens_the_ring_and_letting_go_over_a_key_presses_it() {
        let mut pencil = Pencil::default();
        let at = Pos2::new(500.0, 700.0);
        hover(&mut pencil, at, 0.3);
        let (heard, _) = frame(&mut pencil, 0.31, vec![key(SQUEEZE, true)], false);
        assert!(heard.wake.is_some());
        let (heard, _) = frame(&mut pencil, 0.5, vec![], false);
        assert!(pencil.ring().is_none());
        assert!(heard.wake.is_some_and(|wake| wake > 0.0 && wake < HOLD));
        let (heard, seen) = frame(&mut pencil, 0.32 + HOLD, vec![], false);
        assert!(heard.actions.is_empty());
        assert!(heard.wake.is_none());
        assert_eq!(pencil.ring(), Some(at));
        assert_eq!(seen, vec![Event::PointerMoved(at)], "the pointer is the tip's again");
        // Move over Enter (the top key) and let go.
        let enter = key_rect(at, 0).center();
        let (heard, seen) = frame(&mut pencil, 0.9, vec![Event::PointerMoved(enter)], false);
        assert!(heard.actions.is_empty());
        assert_eq!(seen, vec![Event::PointerMoved(enter)]);
        let (heard, _) = frame(&mut pencil, 1.0, vec![key(SQUEEZE, false)], false);
        assert_eq!(heard.actions, vec![Action::Key { key: Key::Enter, modifiers: Modifiers::NONE }]);
        assert!(pencil.ring().is_none());
    }

    #[test]
    fn letting_a_held_squeeze_go_off_the_keys_leaves_the_ring_open_until_a_press_elsewhere() {
        let mut pencil = Pencil::default();
        let at = Pos2::new(500.0, 700.0);
        hover(&mut pencil, at, 0.3);
        frame(&mut pencil, 0.31, vec![key(SQUEEZE, true)], false);
        frame(&mut pencil, 0.8, vec![], false);
        let (heard, _) = frame(&mut pencil, 0.9, vec![key(SQUEEZE, false)], false);
        assert!(heard.actions.is_empty(), "the dead centre presses nothing");
        assert_eq!(pencil.ring(), Some(at));
        // A tap on a key reaches the app untouched (the ring's own button takes it).
        let shift = key_rect(at, 7).center();
        let (_, seen) = frame(&mut pencil, 1.0, vec![Event::PointerMoved(shift), button(shift, true)], false);
        assert_eq!(seen.len(), 2);
        assert_eq!(pencil.ring(), Some(at));
        frame(&mut pencil, 1.05, vec![button(shift, false), Event::PointerGone], false);
        assert!(pencil.press(Item::Shift).is_none());
        assert!(pencil.held().shift);
        assert!(pencil.ring().is_none());
    }

    #[test]
    fn a_press_away_from_the_open_ring_closes_it_and_goes_through() {
        let mut pencil = Pencil::default();
        hover(&mut pencil, Pos2::new(500.0, 700.0), 0.3);
        frame(&mut pencil, 0.31, vec![key(SQUEEZE, true)], false);
        frame(&mut pencil, 0.8, vec![], false);
        frame(&mut pencil, 0.9, vec![key(SQUEEZE, false)], false);
        assert!(pencil.ring().is_some());
        let away = Pos2::new(900.0, 200.0);
        let (_, seen) = frame(&mut pencil, 1.0, vec![Event::PointerMoved(away), button(away, true)], false);
        assert!(pencil.ring().is_none());
        assert_eq!(seen, vec![Event::PointerMoved(away), button(away, true)]);
    }

    #[test]
    fn a_held_modifier_goes_with_the_next_squeeze_click_and_no_further() {
        let mut pencil = Pencil::default();
        let at = Pos2::new(300.0, 300.0);
        assert!(pencil.press(Item::Shift).is_none());
        assert!(pencil.press(Item::Ctrl).is_none());
        hover(&mut pencil, at, 0.3);
        frame(&mut pencil, 0.31, vec![key(SQUEEZE, true)], false);
        let (heard, _) = frame(&mut pencil, 0.45, vec![key(SQUEEZE, false)], false);
        let both = Modifiers { shift: true, ctrl: true, command: true, ..Modifiers::NONE };
        assert_eq!(heard.actions, vec![Action::Click { at, modifiers: both }]);
        assert!(pencil.held().is_none());
        // egui is told the modifiers are down while held, and once that they are not.
        let (_, seen) = frame(&mut pencil, 0.46, vec![], true);
        assert_eq!(seen, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        let (_, seen) = frame(&mut pencil, 0.47, vec![], true);
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn a_held_modifier_goes_with_the_next_tap_of_the_tip_and_is_let_go_after_it() {
        let mut pencil = Pencil::default();
        let at = Pos2::new(300.0, 300.0);
        assert!(pencil.press(Item::Shift).is_none());
        let (_, seen) = frame(&mut pencil, 1.0, vec![Event::PointerMoved(at), button(at, true)], false);
        assert_eq!(
            seen,
            vec![
                Event::ModifiersChanged(Modifiers::SHIFT),
                Event::PointerMoved(at),
                Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::SHIFT },
            ]
        );
        let (heard, seen) = frame(&mut pencil, 1.1, vec![button(at, false), Event::PointerGone], false);
        assert!(
            seen.contains(&Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::SHIFT }),
            "{seen:?}"
        );
        assert_eq!(heard.wake, Some(0.0), "the next frame lets the modifier go");
        let (_, seen) = frame(&mut pencil, 1.11, vec![], false);
        assert_eq!(seen, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        assert!(pencil.held().is_none());
    }

    #[test]
    fn a_ring_key_takes_the_held_modifiers_with_it() {
        let mut pencil = Pencil::default();
        assert!(pencil.press(Item::Shift).is_none());
        assert_eq!(pencil.press(Item::Tab), Some(Action::Key { key: Key::Tab, modifiers: Modifiers::SHIFT }));
        assert!(pencil.held().is_none());
        assert_eq!(pencil.press(Item::Undo), Some(Action::Key { key: Key::Z, modifiers: CTRL }));
        assert_eq!(pencil.press(Item::Redo), Some(Action::Key { key: Key::Y, modifiers: CTRL }));
    }

    #[test]
    fn a_double_tap_closes_the_ring_then_lets_modifiers_go_then_is_escape() {
        let mut pencil = Pencil::default();
        hover(&mut pencil, Pos2::new(500.0, 700.0), 0.3);
        frame(&mut pencil, 0.31, vec![key(SQUEEZE, true)], false);
        frame(&mut pencil, 0.8, vec![], false);
        frame(&mut pencil, 0.9, vec![key(SQUEEZE, false)], false);
        assert!(pencil.ring().is_some());
        let (heard, _) = frame(&mut pencil, 1.0, vec![key(DOUBLE_TAP, true), key(DOUBLE_TAP, false)], false);
        assert!(heard.actions.is_empty());
        assert!(pencil.ring().is_none());
        assert!(pencil.press(Item::Ctrl).is_none());
        let (heard, _) = frame(&mut pencil, 1.1, vec![key(DOUBLE_TAP, true), key(DOUBLE_TAP, false)], false);
        assert!(heard.actions.is_empty());
        assert!(pencil.held().is_none());
        let (heard, _) = frame(&mut pencil, 1.2, vec![key(DOUBLE_TAP, true), key(DOUBLE_TAP, false)], false);
        assert_eq!(heard.actions, vec![Action::Key { key: Key::Escape, modifiers: Modifiers::NONE }]);
    }

    #[test]
    fn a_squeeze_the_system_takes_back_does_nothing() {
        let mut pencil = Pencil::default();
        let at = Pos2::new(300.0, 300.0);
        hover(&mut pencil, at, 0.3);
        frame(&mut pencil, 0.31, vec![key(SQUEEZE, true)], false);
        let (heard, seen) = frame(&mut pencil, 0.4, vec![key(SQUEEZE_TAKEN, true), key(SQUEEZE_TAKEN, false), key(SQUEEZE, false)], false);
        assert!(heard.actions.is_empty());
        assert!(heard.wake.is_none());
        assert_eq!(seen, vec![Event::PointerMoved(at)], "the pin lifts");
        let (heard, _) = frame(&mut pencil, 2.0, vec![], false);
        assert!(heard.actions.is_empty());
        assert!(pencil.ring().is_none());
    }

    #[test]
    fn a_squeeze_out_of_hover_range_clicks_nothing_and_a_held_one_opens_the_ring_mid_screen() {
        let mut pencil = Pencil::default();
        frame(&mut pencil, 1.0, vec![key(SQUEEZE, true)], false);
        let (heard, _) = frame(&mut pencil, 1.1, vec![key(SQUEEZE, false)], false);
        assert!(heard.actions.is_empty());
        frame(&mut pencil, 2.0, vec![key(SQUEEZE, true)], false);
        frame(&mut pencil, 2.5, vec![], false);
        assert_eq!(pencil.ring(), Some(SCREEN.center()));
    }

    #[test]
    fn the_window_layers_hover_pose_serves_when_no_hover_came_before_the_squeeze() {
        // The window layer sends the squeeze's own hover position just before the key.
        let mut pencil = Pencil::default();
        let at = Pos2::new(640.0, 480.0);
        frame(&mut pencil, 1.0, vec![Event::PointerMoved(at), key(SQUEEZE, true)], false);
        let (heard, _) = frame(&mut pencil, 1.1, vec![key(SQUEEZE, false)], false);
        assert_eq!(heard.actions, vec![Action::Click { at, modifiers: Modifiers::NONE }]);
    }

    #[test]
    fn a_stroke_in_contact_is_never_pinned_and_a_squeeze_in_contact_clicks_nothing() {
        let mut pencil = Pencil::default();
        let a = Pos2::new(100.0, 100.0);
        let b = Pos2::new(180.0, 140.0);
        frame(&mut pencil, 1.0, vec![Event::PointerMoved(a), button(a, true)], false);
        let (heard, seen) = frame(&mut pencil, 1.1, vec![key(SQUEEZE, true), Event::PointerMoved(b)], false);
        assert!(heard.actions.is_empty());
        assert_eq!(seen, vec![Event::PointerMoved(b)]);
        let (heard, _) = frame(&mut pencil, 1.2, vec![key(SQUEEZE, false)], false);
        assert!(heard.actions.is_empty());
    }

    #[test]
    fn the_ring_stays_on_the_screen_and_its_keys_do_not_overlap() {
        for at in [Pos2::new(0.0, 0.0), Pos2::new(5000.0, 5000.0), Pos2::new(3.0, 900.0), Pos2::new(1030.0, 30.0)] {
            let centre = ring_centre(at, SCREEN);
            for index in 0..ITEMS.len() {
                let key = key_rect(centre, index);
                assert!(SCREEN.contains_rect(key), "{at:?}: {key:?}");
                assert!(ring_bounds(centre).contains_rect(key));
                assert_eq!(item_at(centre, key.center()), ITEMS.get(index).copied());
                for other in index + 1..ITEMS.len() {
                    assert!(!key.intersects(key_rect(centre, other)), "keys {index} and {other} overlap");
                }
            }
            assert_eq!(item_at(centre, centre), None, "the centre is dead");
        }
    }

    #[test]
    fn hostile_input_never_panics() {
        let mut pencil = Pencil::default();
        let nan = Pos2::new(f32::NAN, f32::INFINITY);
        for screen in [Rect::NOTHING, Rect::EVERYTHING, Rect::from_min_max(nan, nan), Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 10.0))] {
            for time in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e300] {
                let mut raw = egui::RawInput {
                    time: Some(time),
                    events: vec![
                        Event::PointerMoved(nan),
                        key(SQUEEZE, true),
                        key(DOUBLE_TAP, true),
                        Event::PointerGone,
                        key(SQUEEZE, false),
                        key(SQUEEZE, true),
                    ],
                    ..Default::default()
                };
                let heard = pencil.frame(&mut raw, screen, false);
                assert!(heard.wake.is_none_or(|wake| (0.0..=HOLD).contains(&wake)), "{:?}", heard.wake);
                let mut raw = egui::RawInput { time: Some(time + 1.0), ..Default::default() };
                pencil.frame(&mut raw, screen, true);
                if let Some(centre) = pencil.ring() {
                    assert!(centre.x.is_finite() && centre.y.is_finite(), "{screen:?}: {centre:?}");
                }
                let _ = pencil.readout();
            }
        }
    }

    #[test]
    fn actions_are_the_apps_synthetic_input() {
        let at = Pos2::new(1.0, 2.0);
        let click = Action::Click { at, modifiers: Modifiers::SHIFT }.events();
        assert_eq!(click.len(), 5);
        assert_eq!(click.first(), Some(&Event::ModifiersChanged(Modifiers::SHIFT)));
        assert_eq!(click.iter().filter(|event| matches!(event, Event::PointerButton { .. })).count(), 2);
        assert_eq!(click.last(), Some(&Event::ModifiersChanged(Modifiers::NONE)));
        let escape = Action::Key { key: Key::Escape, modifiers: Modifiers::NONE }.events();
        assert_eq!(escape.iter().filter(|event| matches!(event, Event::Key { key: Key::Escape, .. })).count(), 2);
    }
}
