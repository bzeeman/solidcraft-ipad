//! The number pad: the iPad's keys for values, in place of the system's keyboard.
//!
//! An expression box (a dimension, a value in a tool's options, a parameter) that takes the focus
//! gets this pad beside it, and the system's keyboard stays down: the pad is small, sits next to
//! the box instead of over half the screen, and its keys are large enough for the Pencil's tip or
//! a finger. It has the digits, the point, the four operators and parentheses (a box takes
//! expressions), the caret's two moves, Delete, Esc and Enter, and a key for each of the design's
//! named parameters. Its ABC key brings up the system's keyboard instead, for that box until it
//! loses the focus; the 123 key beside the box then brings the pad back. Every other text box (a
//! name, a search) gets the system's keyboard as before.
//!
//! The pad reads its presses out of a frame's input before the app sees them ([`Pad::input`]), and
//! puts back what the key types. So the box never sees a press away from it, and keeps the focus
//! and its caret. The press is the key's: it types at once, and its release and any movement
//! until then are dropped, so a slip of the tip types nothing else.
//!
//! Nothing here is specific to iOS: it reads and writes egui's input only, so it is tested on any
//! platform.

use egui::{Event, Id, Key as EguiKey, Modifiers, PointerButton, Rect, Vec2, pos2, vec2};

/// A key's side (points): more than Apple's 44 for a finger, so a Pencil's tip lands without care.
pub const KEY: f32 = 52.0;
/// The space between two keys, and between the keys and the pad's edge.
pub const GAP: f32 = 6.0;
/// A parameter's key: as high as the other keys are less tall, two to a row.
pub const NAME_HEIGHT: f32 = 36.0;
/// At most this many of the design's parameters get a key.
pub const NAMES: usize = 6;
/// The space between the pad and the box it types into.
pub const BESIDE: f32 = 12.0;
/// How long a pressed key stays lit (seconds).
pub const LIT: f64 = 0.15;
/// The 123 key beside a box that has the system's keyboard.
pub const BACK: Vec2 = vec2(52.0, 36.0);

/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Types this text (an operator's key shows one sign and types another, see [`Key::label`]).
    Type(&'static str),
    /// Types the name of the design's parameter at this place in the pad's list.
    Name(usize),
    Delete,
    Left,
    Right,
    Escape,
    Enter,
    /// The system's keyboard instead of the pad, for this box.
    Keyboard,
    /// The pad again, in place of the system's keyboard.
    Pad,
}

/// The keys under the parameters, a row at a time, with how many columns each spans.
const ROWS: [&[(Key, u8)]; 5] = [
    &[(Key::Type("7"), 1), (Key::Type("8"), 1), (Key::Type("9"), 1), (Key::Type("("), 1), (Key::Type(")"), 1)],
    &[(Key::Type("4"), 1), (Key::Type("5"), 1), (Key::Type("6"), 1), (Key::Type("*"), 1), (Key::Type("/"), 1)],
    &[(Key::Type("1"), 1), (Key::Type("2"), 1), (Key::Type("3"), 1), (Key::Type("+"), 1), (Key::Type("-"), 1)],
    &[(Key::Type("0"), 1), (Key::Type("."), 1), (Key::Left, 1), (Key::Right, 1), (Key::Delete, 1)],
    &[(Key::Keyboard, 1), (Key::Escape, 1), (Key::Enter, 3)],
];
/// Columns of [`ROWS`].
const COLUMNS: u8 = 5;

impl Key {
    /// What the key shows. The names' keys show the name, which the pad looks up.
    pub fn label(self) -> &'static str {
        match self {
            Self::Type("*") => "\u{00d7}",
            Self::Type("/") => "\u{00f7}",
            Self::Type("-") => "\u{2212}",
            Self::Type(text) => text,
            Self::Name(_) => "",
            Self::Delete => "Del",
            Self::Left => "\u{2190}",
            Self::Right => "\u{2192}",
            Self::Escape => "Esc",
            Self::Enter => "Enter",
            Self::Keyboard => "ABC",
            Self::Pad => "123",
        }
    }

    /// The input the key gives the box: text, or a key's press and release.
    fn events(self, names: &[String]) -> Vec<Event> {
        let press = |key| {
            vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
            ]
        };
        match self {
            Self::Type(text) => vec![Event::Text(text.to_owned())],
            Self::Name(index) => names.get(index).map(|name| vec![Event::Text(name.clone())]).unwrap_or_default(),
            Self::Delete => press(EguiKey::Backspace),
            Self::Left => press(EguiKey::ArrowLeft),
            Self::Right => press(EguiKey::ArrowRight),
            Self::Escape => press(EguiKey::Escape),
            Self::Enter => press(EguiKey::Enter),
            Self::Keyboard | Self::Pad => Vec::new(),
        }
    }
}

/// Where the pad's keys are, for a box at `field` with these parameters, inside `content`.
/// The pad goes to the right of the box, or else to its left, or else below or above it; its top
/// level with the box's where it can be. Returns the pad's outline and each key's place.
pub fn layout(field: Rect, names: usize, content: Rect) -> (Rect, Vec<(Rect, Key)>) {
    let names = names.min(NAMES);
    let name_rows = names.div_ceil(2);
    let inner_width = f32::from(COLUMNS) * KEY + f32::from(COLUMNS - 1) * GAP;
    let names_height = name_rows as f32 * (NAME_HEIGHT + GAP);
    let keys_height = ROWS.len() as f32 * KEY + (ROWS.len() - 1) as f32 * GAP;
    let size = vec2(inner_width + 2.0 * GAP, names_height + keys_height + 2.0 * GAP);

    let fits_x = |x: f32| x >= content.left() && x + size.x <= content.right();
    let right = field.right() + BESIDE;
    let left = field.left() - BESIDE - size.x;
    let min = if fits_x(right) || fits_x(left) {
        let x = if fits_x(right) { right } else { left };
        pos2(x, field.top())
    } else {
        let below = field.bottom() + BESIDE;
        let y = if below + size.y <= content.bottom() { below } else { field.top() - BESIDE - size.y };
        pos2(field.left(), y)
    };
    let min = pos2(clamp(min.x, content.left(), content.right() - size.x), clamp(min.y, content.top(), content.bottom() - size.y));
    let outline = Rect::from_min_size(min, size);

    let mut keys = Vec::new();
    let origin = min + vec2(GAP, GAP);
    let name_width = (inner_width - GAP) / 2.0;
    for index in 0..names {
        let at = origin + vec2((index % 2) as f32 * (name_width + GAP), (index / 2) as f32 * (NAME_HEIGHT + GAP));
        keys.push((Rect::from_min_size(at, vec2(name_width, NAME_HEIGHT)), Key::Name(index)));
    }
    for (row, row_keys) in ROWS.iter().enumerate() {
        let mut column = 0u8;
        for &(key, span) in row_keys.iter() {
            let at = origin + vec2(f32::from(column) * (KEY + GAP), names_height + row as f32 * (KEY + GAP));
            let width = f32::from(span) * KEY + f32::from(span.saturating_sub(1)) * GAP;
            keys.push((Rect::from_min_size(at, vec2(width, KEY)), key));
            column = column.saturating_add(span);
        }
    }
    (outline, keys)
}

/// The 123 key's place beside a box that has the system's keyboard: to its right, or its left.
pub fn back_key(field: Rect, content: Rect) -> Rect {
    let x = if field.right() + BESIDE + BACK.x <= content.right() { field.right() + BESIDE } else { field.left() - BESIDE - BACK.x };
    let y = field.center().y - BACK.y / 2.0;
    Rect::from_min_size(pos2(clamp(x, content.left(), content.right() - BACK.x), clamp(y, content.top(), content.bottom() - BACK.y)), BACK)
}

/// `value` within `low..=high`, or `low` when there is no room (a screen smaller than the pad).
fn clamp(value: f32, low: f32, high: f32) -> f32 {
    if high < low { low } else { value.clamp(low, high) }
}

/// What the pad shows this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    /// The pad's outline, or the 123 key's.
    pub outline: Rect,
    pub keys: Vec<(Rect, Key)>,
    /// The parameters the names' keys type, in the keys' order.
    pub names: Vec<String>,
}

/// The pad from one frame to the next.
#[derive(Debug, Default)]
pub struct Pad {
    /// The expression box that had the focus when the app last drew.
    field: Option<Id>,
    /// The box that has the system's keyboard instead of the pad, while it keeps the focus.
    keyboard_for: Option<Id>,
    /// What was drawn last frame: where this frame's presses land.
    shown: Option<Shown>,
    /// A press on the pad is down: its movement and its release are the pad's.
    pressing: bool,
    /// The key pressed last, and when, to light it.
    lit: Option<(Key, f64)>,
}

impl Pad {
    /// Takes the presses on the pad out of a frame's input and puts in what their keys type.
    /// Returns whether a key was pressed (the frame then needs drawing again to light it).
    pub fn input(&mut self, raw: &mut egui::RawInput) -> bool {
        let Some(shown) = self.shown.clone() else {
            self.pressing = false;
            return false;
        };
        let now = raw.time.unwrap_or_default();
        let mut typed = false;
        let mut events = Vec::with_capacity(raw.events.len());
        for event in std::mem::take(&mut raw.events) {
            match event {
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } if shown.outline.contains(pos) => {
                    self.pressing = true;
                    if let Some(&(_, key)) = shown.keys.iter().find(|(rect, _)| rect.contains(pos)) {
                        events.extend(self.press(key, &shown.names, now));
                        typed = true;
                    }
                }
                Event::PointerButton { pressed: false, .. } if self.pressing => self.pressing = false,
                Event::PointerMoved(pos) if self.pressing || shown.outline.contains(pos) => {}
                other => events.push(other),
            }
        }
        raw.events = events;
        typed
    }

    fn press(&mut self, key: Key, names: &[String], now: f64) -> Vec<Event> {
        self.lit = Some((key, now));
        match key {
            Key::Keyboard => self.keyboard_for = self.field,
            Key::Pad => self.keyboard_for = None,
            _ => {}
        }
        key.events(names)
    }

    /// Decides what shows this frame, after the app has drawn: `field` is the expression box that
    /// has the focus and `field_rect` where it is. Returns what to draw, and whether the system's
    /// keyboard may come up (the pad stands in for it).
    pub fn frame(&mut self, field: Option<(Id, Rect)>, names: Vec<String>, content: Rect) -> (Option<&Shown>, bool) {
        self.field = field.map(|(id, _)| id);
        if self.keyboard_for.is_some() && self.keyboard_for != self.field {
            self.keyboard_for = None;
        }
        self.shown = match field {
            None => None,
            Some((_, rect)) if self.keyboard_for.is_some() => {
                let back = back_key(rect, content);
                Some(Shown { outline: back, keys: vec![(back, Key::Pad)], names: Vec::new() })
            }
            Some((_, rect)) => {
                let names: Vec<String> = names.into_iter().take(NAMES).collect();
                let (outline, keys) = layout(rect, names.len(), content);
                Some(Shown { outline, keys, names })
            }
        };
        let keyboard = self.shown.as_ref().is_none_or(|_| self.keyboard_for.is_some());
        (self.shown.as_ref(), keyboard)
    }

    /// The key to show lit at `now`, and how long until it goes out.
    pub fn lit(&self, now: f64) -> Option<(Key, f64)> {
        self.lit.and_then(|(key, at)| (now - at < LIT).then_some((key, LIT - (now - at))))
    }
}

/// A point inside the first key that does `key` in `shown`, for tests and self-tests.
#[cfg(test)]
fn at(shown: &Shown, key: Key) -> egui::Pos2 {
    shown.keys.iter().find(|(_, k)| *k == key).map(|(rect, _)| rect.center()).unwrap_or(egui::Pos2::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENT: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(1366.0, 1000.0) };

    fn field_at(x: f32, y: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(120.0, 28.0))
    }

    fn tap(at: egui::Pos2) -> Vec<Event> {
        vec![
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        ]
    }

    fn run(pad: &mut Pad, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { events, time: Some(1.0), ..Default::default() };
        pad.input(&mut raw);
        raw.events
    }

    fn shown(pad: &mut Pad, id: Id, field: Rect, names: &[&str]) -> Shown {
        let (shown, _) = pad.frame(Some((id, field)), names.iter().map(|s| (*s).to_owned()).collect(), CONTENT);
        shown.cloned().expect("the pad shows")
    }

    #[test]
    fn the_pad_sits_beside_the_box_inside_the_screen() {
        let field = field_at(400.0, 300.0);
        let (outline, keys) = layout(field, 0, CONTENT);
        assert_eq!(outline.left(), field.right() + BESIDE, "to the right of the box");
        assert_eq!(outline.top(), field.top(), "level with the box");
        assert!(!outline.intersects(field));
        assert!(keys.iter().all(|(rect, _)| outline.contains_rect(*rect)));
        assert!(keys.iter().all(|(rect, _)| rect.height() >= 44.0), "every key is large enough to hit");

        // At the right edge it goes left; at the bottom it rises to stay on screen.
        let field = field_at(1200.0, 950.0);
        let (outline, _) = layout(field, 4, CONTENT);
        assert_eq!(outline.right(), field.left() - BESIDE);
        assert!(CONTENT.contains_rect(outline));
        assert!(!outline.intersects(field));

        // A box as wide as the screen: the pad goes below it, or above it near the bottom.
        let wide = Rect::from_min_size(pos2(0.0, 100.0), vec2(1366.0, 28.0));
        assert_eq!(layout(wide, 0, CONTENT).0.top(), wide.bottom() + BESIDE);
        let wide = Rect::from_min_size(pos2(0.0, 900.0), vec2(1366.0, 28.0));
        assert_eq!(layout(wide, 0, CONTENT).0.bottom(), wide.top() - BESIDE);
    }

    #[test]
    fn no_two_keys_overlap_and_every_key_has_its_place() {
        let (_, keys) = layout(field_at(100.0, 100.0), 9, CONTENT);
        assert_eq!(keys.iter().filter(|(_, k)| matches!(k, Key::Name(_))).count(), NAMES, "at most six names");
        for (i, (a, _)) in keys.iter().enumerate() {
            for (b, _) in &keys[i + 1..] {
                assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{a:?} overlaps {b:?}");
            }
        }
        for key in [Key::Type("0"), Key::Type("9"), Key::Type("."), Key::Delete, Key::Enter, Key::Escape, Key::Keyboard] {
            assert!(keys.iter().any(|(_, k)| *k == key), "{key:?} is on the pad");
        }
    }

    #[test]
    fn keys_type_into_the_box_and_the_press_never_reaches_it() {
        let mut pad = Pad::default();
        let id = Id::unique("box");
        let shown = shown(&mut pad, id, field_at(400.0, 300.0), &["width", "gap"]);
        let mut typed = Vec::new();
        for key in [Key::Type("1"), Key::Type("2"), Key::Type("/"), Key::Name(1), Key::Delete, Key::Enter] {
            typed.extend(run(&mut pad, tap(at(&shown, key))));
        }
        assert!(typed.iter().all(|e| !matches!(e, Event::PointerButton { .. } | Event::PointerMoved(_))), "the box saw no press: {typed:?}");
        let text: String = typed.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
        assert_eq!(text, "12/gap");
        let pressed: Vec<EguiKey> = typed.iter().filter_map(|e| if let Event::Key { key, pressed: true, .. } = e { Some(*key) } else { None }).collect();
        assert_eq!(pressed, [EguiKey::Backspace, EguiKey::Enter]);

        // A press elsewhere is the app's.
        let away = run(&mut pad, tap(pos2(10.0, 10.0)));
        assert_eq!(away.len(), 3);
    }

    #[test]
    fn a_press_on_the_pad_keeps_its_release_and_its_slide() {
        let mut pad = Pad::default();
        let shown = shown(&mut pad, Id::unique("box"), field_at(400.0, 300.0), &[]);
        let key = at(&shown, Key::Type("5"));
        let down = run(&mut pad, vec![Event::PointerButton { pos: key, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        assert_eq!(down, vec![Event::Text("5".into())]);
        // The tip slides off the pad and lifts there: still the pad's.
        let off = pos2(10.0, 10.0);
        let rest = run(
            &mut pad,
            vec![Event::PointerMoved(off), Event::PointerButton { pos: off, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
        );
        assert!(rest.is_empty(), "{rest:?}");
        // The next press elsewhere is the app's again.
        assert_eq!(run(&mut pad, tap(off)).len(), 3);
    }

    #[test]
    fn abc_brings_the_keyboard_for_that_box_and_123_takes_it_away() {
        let mut pad = Pad::default();
        let id = Id::unique("box");
        let field = field_at(400.0, 300.0);
        let (_, keyboard) = pad.frame(Some((id, field)), Vec::new(), CONTENT);
        assert!(!keyboard, "the pad stands in for the system's keyboard");
        let pad_shown = shown(&mut pad, id, field, &[]);
        run(&mut pad, tap(at(&pad_shown, Key::Keyboard)));
        let (back, keyboard) = pad.frame(Some((id, field)), Vec::new(), CONTENT);
        assert!(keyboard);
        let back = back.cloned().expect("the 123 key shows");
        assert_eq!(back.keys.len(), 1, "only the 123 key");
        assert!(!back.outline.intersects(field));

        // Another box gets the pad again; so does the first once it has lost the focus.
        let (_, keyboard) = pad.frame(Some((Id::unique("other"), field)), Vec::new(), CONTENT);
        assert!(!keyboard);
        let (_, keyboard) = pad.frame(Some((id, field)), Vec::new(), CONTENT);
        assert!(!keyboard, "the keyboard was for that focus only");

        run(&mut pad, tap(at(&pad_shown, Key::Keyboard)));
        pad.frame(Some((id, field)), Vec::new(), CONTENT);
        run(&mut pad, tap(back.outline.center()));
        let (_, keyboard) = pad.frame(Some((id, field)), Vec::new(), CONTENT);
        assert!(!keyboard, "123 brought the pad back");
    }

    #[test]
    fn without_an_expression_box_the_pad_takes_nothing() {
        let mut pad = Pad::default();
        let (shown, keyboard) = pad.frame(None, vec!["width".into()], CONTENT);
        assert!(shown.is_none());
        assert!(keyboard, "a name or a search box gets the system's keyboard");
        let at = pos2(500.0, 320.0);
        assert_eq!(run(&mut pad, tap(at)).len(), 3);
    }

    #[test]
    fn a_pressed_key_is_lit_for_a_moment() {
        let mut pad = Pad::default();
        let shown = shown(&mut pad, Id::unique("box"), field_at(400.0, 300.0), &[]);
        run(&mut pad, tap(at(&shown, Key::Type("7"))));
        assert_eq!(pad.lit(1.0).map(|(k, _)| k), Some(Key::Type("7")));
        assert!(pad.lit(1.0 + LIT + 0.01).is_none());
    }

    #[test]
    fn a_name_key_beyond_the_list_types_nothing() {
        assert!(Key::Name(3).events(&["a".into()]).is_empty());
    }
}
