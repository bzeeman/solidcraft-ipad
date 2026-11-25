//! The iPad's own keys for text boxes, in place of the system's keyboard: a number pad and a
//! compact keyboard.
//!
//! A text box that takes the focus gets keys of ours beside it, and the system's keyboard stays
//! down. An expression box (a dimension, a value in a tool's options, a parameter) gets the number
//! pad: the digits, the point, the four operators and parentheses, the caret's two moves, Delete,
//! Esc, Enter, and a key for each of the design's named parameters (a box takes `width/2`). Any
//! other text box (a name, a search) gets the compact keyboard: letters, a layer of digits and
//! signs, space, Delete and Return, about a third of the screen wide. Every key is large enough
//! for the Pencil's tip or a finger.
//!
//! The two reach each other (ABC on the pad, 123 on the keyboard), and Full on either brings up
//! the system's whole keyboard instead, for that box until it loses the focus; a key beside the
//! box then brings ours back. Each has a handle along its top: drag it and the pad or the keyboard
//! stays where it is put, for every box after, until the app quits.
//!
//! The keys read their presses out of a frame's input before the app sees them ([`Keys::input`]),
//! and put back what the key types. So the box never sees a press away from it, and keeps the
//! focus and its caret. The press is the key's: it types at once, and its release and any
//! movement until then are dropped, so a slip of the tip types nothing else.
//!
//! Nothing here is specific to iOS: it reads and writes egui's input only, so it is tested on any
//! platform.

use egui::{Event, Id, Key as EguiKey, Modifiers, PointerButton, Pos2, Rect, Vec2, pos2, vec2};

/// The space between two keys, and between the keys and the edge.
pub const GAP: f32 = 6.0;
/// The handle along the top that drags the pad or the keyboard.
pub const HANDLE: f32 = 22.0;
/// A parameter's key on the pad: as high as the other keys are less tall, two to a row.
pub const NAME_HEIGHT: f32 = 36.0;
/// At most this many of the design's parameters get a key.
pub const NAMES: usize = 6;
/// The space between the keys and the box they type into.
pub const BESIDE: f32 = 12.0;
/// How long a pressed key stays lit (seconds).
pub const LIT: f64 = 0.15;
/// The key beside a box that has the system's keyboard, which brings ours back.
pub const BACK: Vec2 = vec2(52.0, 36.0);

/// Which keys show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// The number pad.
    Pad,
    /// The compact keyboard's letters.
    Letters,
    /// The compact keyboard's digits and signs.
    Signs,
}

impl Layout {
    /// The pad and the keyboard are dragged apart: the keyboard's two layers share a place.
    fn place(self) -> usize {
        match self {
            Self::Pad => 0,
            Self::Letters | Self::Signs => 1,
        }
    }

    /// A key's width and height (points), and its parts of a row: the pad has five keys to a row,
    /// the keyboard ten. A key's place is counted in half keys, so a row can sit half a key in.
    fn key(self) -> (f32, f32, u8) {
        match self {
            Self::Pad => (52.0, 52.0, 10),
            Self::Letters | Self::Signs => (40.0, 46.0, 20),
        }
    }
}

/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Types this text (an operator's key shows one sign and types another, see [`Key::label`]).
    Type(&'static str),
    /// Types this letter, as a capital after Shift.
    Letter(char),
    /// Types the name of the design's parameter at this place in the pad's list.
    Name(usize),
    /// The next letter is a capital.
    Shift,
    Delete,
    Left,
    Right,
    Escape,
    Enter,
    /// These keys instead.
    Show(Layout),
    /// The system's whole keyboard instead, for this box.
    Full,
    /// Drags the pad or the keyboard.
    Handle,
}

/// A row of keys: how many half keys in it starts, and each key with its width in half keys.
type Row = (u8, &'static [(Key, u8)]);

const PAD: [Row; 5] = [
    (0, &[(Key::Type("7"), 2), (Key::Type("8"), 2), (Key::Type("9"), 2), (Key::Type("("), 2), (Key::Type(")"), 2)]),
    (0, &[(Key::Type("4"), 2), (Key::Type("5"), 2), (Key::Type("6"), 2), (Key::Type("*"), 2), (Key::Type("/"), 2)]),
    (0, &[(Key::Type("1"), 2), (Key::Type("2"), 2), (Key::Type("3"), 2), (Key::Type("+"), 2), (Key::Type("-"), 2)]),
    (0, &[(Key::Type("0"), 2), (Key::Type("."), 2), (Key::Left, 2), (Key::Right, 2), (Key::Delete, 2)]),
    (0, &[(Key::Show(Layout::Letters), 2), (Key::Full, 2), (Key::Escape, 2), (Key::Enter, 4)]),
];

/// The keyboard's bottom row; its first key (`Show`) is filled in for the box (see [`rows`]).
const BOTTOM: [(Key, u8); 5] = [(Key::Show(Layout::Pad), 3), (Key::Full, 2), (Key::Type(" "), 9), (Key::Escape, 2), (Key::Enter, 4)];

const LETTERS: [Row; 3] = [
    (
        0,
        &[
            (Key::Letter('q'), 2),
            (Key::Letter('w'), 2),
            (Key::Letter('e'), 2),
            (Key::Letter('r'), 2),
            (Key::Letter('t'), 2),
            (Key::Letter('y'), 2),
            (Key::Letter('u'), 2),
            (Key::Letter('i'), 2),
            (Key::Letter('o'), 2),
            (Key::Letter('p'), 2),
        ],
    ),
    (
        1,
        &[
            (Key::Letter('a'), 2),
            (Key::Letter('s'), 2),
            (Key::Letter('d'), 2),
            (Key::Letter('f'), 2),
            (Key::Letter('g'), 2),
            (Key::Letter('h'), 2),
            (Key::Letter('j'), 2),
            (Key::Letter('k'), 2),
            (Key::Letter('l'), 2),
        ],
    ),
    (
        0,
        &[
            (Key::Shift, 3),
            (Key::Letter('z'), 2),
            (Key::Letter('x'), 2),
            (Key::Letter('c'), 2),
            (Key::Letter('v'), 2),
            (Key::Letter('b'), 2),
            (Key::Letter('n'), 2),
            (Key::Letter('m'), 2),
            (Key::Delete, 3),
        ],
    ),
];

const SIGNS: [Row; 3] = [
    (
        0,
        &[
            (Key::Type("1"), 2),
            (Key::Type("2"), 2),
            (Key::Type("3"), 2),
            (Key::Type("4"), 2),
            (Key::Type("5"), 2),
            (Key::Type("6"), 2),
            (Key::Type("7"), 2),
            (Key::Type("8"), 2),
            (Key::Type("9"), 2),
            (Key::Type("0"), 2),
        ],
    ),
    (
        1,
        &[
            (Key::Type("-"), 2),
            (Key::Type("/"), 2),
            (Key::Type(":"), 2),
            (Key::Type(";"), 2),
            (Key::Type("("), 2),
            (Key::Type(")"), 2),
            (Key::Type("&"), 2),
            (Key::Type("@"), 2),
            (Key::Type("\""), 2),
        ],
    ),
    (
        0,
        &[
            (Key::Type("+"), 3),
            (Key::Type("."), 2),
            (Key::Type(","), 2),
            (Key::Type("?"), 2),
            (Key::Type("!"), 2),
            (Key::Type("'"), 2),
            (Key::Type("_"), 2),
            (Key::Type("#"), 2),
            (Key::Delete, 3),
        ],
    ),
];

impl Key {
    /// What the key shows; `shift` is whether the next letter is a capital. The names' keys show
    /// the name, which the caller looks up.
    pub fn label(self, shift: bool) -> String {
        match self {
            Self::Type("*") => "\u{00d7}".into(),
            Self::Type("/") => "\u{00f7}".into(),
            Self::Type("-") => "\u{2212}".into(),
            Self::Type(" ") => "space".into(),
            Self::Type(text) => text.into(),
            Self::Letter(letter) if shift => letter.to_uppercase().collect(),
            Self::Letter(letter) => letter.into(),
            Self::Name(_) | Self::Handle => String::new(),
            Self::Shift => "Shift".into(),
            Self::Delete => "Del".into(),
            Self::Left => "\u{2190}".into(),
            Self::Right => "\u{2192}".into(),
            Self::Escape => "Esc".into(),
            Self::Enter => "Enter".into(),
            Self::Show(Layout::Letters) => "ABC".into(),
            Self::Show(Layout::Pad | Layout::Signs) => "123".into(),
            Self::Full => "Full".into(),
        }
    }

    /// Whether the key types something one reads (its label is drawn larger).
    pub fn types(self) -> bool {
        matches!(self, Self::Type(text) if text != " ") || matches!(self, Self::Letter(_))
    }

    /// The input the key gives the box: text, or a key's press and release.
    fn events(self, names: &[String], shift: bool) -> Vec<Event> {
        let press = |key| {
            vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
            ]
        };
        match self {
            Self::Type(text) => vec![Event::Text(text.to_owned())],
            Self::Letter(_) => vec![Event::Text(self.label(shift))],
            Self::Name(index) => names.get(index).map(|name| vec![Event::Text(name.clone())]).unwrap_or_default(),
            Self::Delete => press(EguiKey::Backspace),
            Self::Left => press(EguiKey::ArrowLeft),
            Self::Right => press(EguiKey::ArrowRight),
            Self::Escape => press(EguiKey::Escape),
            Self::Enter => press(EguiKey::Enter),
            Self::Shift | Self::Show(_) | Self::Full | Self::Handle => Vec::new(),
        }
    }
}

/// The rows of `layout`. The keyboard's 123 goes to the pad from an expression box, and to the
/// digits and signs from any other.
fn rows(layout: Layout, expression: bool) -> Vec<(u8, Vec<(Key, u8)>)> {
    let mut bottom = BOTTOM.to_vec();
    if let Some(first) = bottom.first_mut() {
        first.0 = Key::Show(match layout {
            Layout::Signs => Layout::Letters,
            _ if expression => Layout::Pad,
            _ => Layout::Signs,
        });
    }
    let own = |rows: &[Row]| rows.iter().map(|(offset, keys)| (*offset, keys.to_vec())).collect::<Vec<_>>();
    match layout {
        Layout::Pad => own(&PAD),
        Layout::Letters => own(&LETTERS).into_iter().chain([(0, bottom)]).collect(),
        Layout::Signs => own(&SIGNS).into_iter().chain([(0, bottom)]).collect(),
    }
}

/// Where the keys of `layout` go for a box at `field`, with these many parameters' keys (the pad
/// only), inside `content`. `placed` is where the handle last left them; without it the pad goes
/// to the right of the box, or else to its left, or else below or above it, and the keyboard
/// below the box, or else above it. Returns the outline and each key's place, the handle first.
pub fn layout(layout: Layout, expression: bool, field: Rect, names: usize, content: Rect, placed: Option<Pos2>) -> (Rect, Vec<(Rect, Key)>) {
    let names = if layout == Layout::Pad { names.min(NAMES) } else { 0 };
    let (key_width, key_height, halves) = layout.key();
    let half = (key_width + GAP) / 2.0;
    let rows = rows(layout, expression);
    let inner_width = f32::from(halves) * half - GAP;
    let names_height = names.div_ceil(2) as f32 * (NAME_HEIGHT + GAP);
    let keys_height = rows.len() as f32 * (key_height + GAP) - GAP;
    let size = vec2(inner_width + 2.0 * GAP, HANDLE + names_height + keys_height + GAP);

    let fits_x = |x: f32| x >= content.left() && x + size.x <= content.right();
    let below = field.bottom() + BESIDE;
    let below_or_above = || pos2(field.left(), if below + size.y <= content.bottom() { below } else { field.top() - BESIDE - size.y });
    let min = match placed {
        Some(at) => at,
        None if layout == Layout::Pad => {
            let (right, left) = (field.right() + BESIDE, field.left() - BESIDE - size.x);
            if fits_x(right) {
                pos2(right, field.top())
            } else if fits_x(left) {
                pos2(left, field.top())
            } else {
                below_or_above()
            }
        }
        None => below_or_above(),
    };
    let outline = Rect::from_min_size(clamp_into(min, size, content), size);

    let mut keys = vec![(Rect::from_min_size(outline.min, vec2(size.x, HANDLE)), Key::Handle)];
    let origin = outline.min + vec2(GAP, HANDLE);
    let name_width = (inner_width - GAP) / 2.0;
    for index in 0..names {
        let at = origin + vec2((index % 2) as f32 * (name_width + GAP), (index / 2) as f32 * (NAME_HEIGHT + GAP));
        keys.push((Rect::from_min_size(at, vec2(name_width, NAME_HEIGHT)), Key::Name(index)));
    }
    for (row, (offset, row_keys)) in rows.iter().enumerate() {
        let mut column = *offset;
        for &(key, span) in row_keys {
            let at = origin + vec2(f32::from(column) * half, names_height + row as f32 * (key_height + GAP));
            keys.push((Rect::from_min_size(at, vec2(f32::from(span) * half - GAP, key_height)), key));
            column = column.saturating_add(span);
        }
    }
    (outline, keys)
}

/// The key beside a box that has the system's keyboard: to its right, or its left.
pub fn back_key(field: Rect, content: Rect) -> Rect {
    let x = if field.right() + BESIDE + BACK.x <= content.right() { field.right() + BESIDE } else { field.left() - BESIDE - BACK.x };
    Rect::from_min_size(clamp_into(pos2(x, field.center().y - BACK.y / 2.0), BACK, content), BACK)
}

/// `min` moved so that a rectangle of `size` there is inside `content`, as far as it fits.
fn clamp_into(min: Pos2, size: Vec2, content: Rect) -> Pos2 {
    let clamp = |value: f32, low: f32, high: f32| if high < low { low } else { value.clamp(low, high) };
    pos2(clamp(min.x, content.left(), content.right() - size.x), clamp(min.y, content.top(), content.bottom() - size.y))
}

/// What shows for a box, as the box's focus goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Ours(Layout),
    /// The system's keyboard; the key beside the box brings back these keys.
    System(Layout),
}

/// The box that has the focus: which, where, and whether it takes an expression.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    pub id: Id,
    pub rect: Rect,
    pub expression: bool,
}

/// What shows this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    /// The outline of the keys, or of the key beside a box that has the system's keyboard.
    pub outline: Rect,
    pub keys: Vec<(Rect, Key)>,
    /// The parameters the names' keys type, in the keys' order.
    pub names: Vec<String>,
    /// The next letter is a capital.
    pub shift: bool,
}

impl Shown {
    /// Whether these are keys of ours, not the one beside a box with the system's keyboard.
    pub fn ours(&self) -> bool {
        self.keys.len() > 1
    }
}

/// A drag of the handle: which keys, and where in them it took hold.
#[derive(Clone, Copy, Debug)]
struct Drag {
    place: usize,
    grip: Vec2,
}

/// The keys from one frame to the next.
#[derive(Debug, Default)]
pub struct Keys {
    /// The box that had the focus when the app last drew, and what it has.
    field: Option<(Id, Choice)>,
    /// What was drawn last frame: where this frame's presses land.
    shown: Option<Shown>,
    /// Where the handle left the pad and the keyboard (see [`Layout::place`]).
    placed: [Option<Pos2>; 2],
    /// A press on the keys is down: its movement and its release are theirs.
    pressing: bool,
    drag: Option<Drag>,
    shift: bool,
    /// The key pressed last, and when, to light it.
    lit: Option<(Key, f64)>,
}

impl Keys {
    /// Takes the presses on the keys out of a frame's input and puts in what they type, and moves
    /// the keys while their handle is dragged. Returns whether the keys need drawing again.
    pub fn input(&mut self, raw: &mut egui::RawInput) -> bool {
        let Some(shown) = self.shown.clone() else {
            (self.pressing, self.drag) = (false, None);
            return false;
        };
        let now = raw.time.unwrap_or_default();
        let mut changed = false;
        let mut events = Vec::with_capacity(raw.events.len());
        for event in std::mem::take(&mut raw.events) {
            match event {
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } if shown.outline.contains(pos) => {
                    self.pressing = true;
                    if let Some(&(rect, key)) = shown.keys.iter().find(|(rect, _)| rect.contains(pos)) {
                        events.extend(self.press(key, &shown, now));
                        if key == Key::Handle
                            && let Some((_, Choice::Ours(layout))) = self.field
                        {
                            self.drag = Some(Drag { place: layout.place(), grip: pos - rect.min });
                        }
                        changed = true;
                    }
                }
                Event::PointerButton { pressed: false, .. } if self.pressing => (self.pressing, self.drag) = (false, None),
                Event::PointerMoved(pos) if self.pressing || shown.outline.contains(pos) => {
                    if let Some(drag) = self.drag
                        && let Some(slot) = self.placed.get_mut(drag.place)
                    {
                        *slot = Some(pos - drag.grip);
                        changed = true;
                    }
                }
                other => events.push(other),
            }
        }
        raw.events = events;
        changed
    }

    fn press(&mut self, key: Key, shown: &Shown, now: f64) -> Vec<Event> {
        self.lit = (key != Key::Handle).then_some((key, now));
        let events = key.events(&shown.names, self.shift);
        match key {
            Key::Shift => self.shift = !self.shift,
            Key::Letter(_) => self.shift = false,
            _ => {}
        }
        if let Some((_, choice)) = &mut self.field {
            match (key, *choice) {
                (Key::Show(layout), _) => *choice = Choice::Ours(layout),
                (Key::Full, Choice::Ours(layout)) => *choice = Choice::System(layout),
                _ => {}
            }
        }
        events
    }

    /// Decides what shows this frame, after the app has drawn: `field` is the text box that has
    /// the focus, if one does, and `names` the design's parameters. Returns what to draw, and
    /// whether the system's keyboard may come up (it may only when ours are not showing).
    pub fn frame(&mut self, field: Option<Field>, names: Vec<String>, content: Rect) -> (Option<&Shown>, bool) {
        let Some(field) = field else {
            (self.field, self.shown, self.shift) = (None, None, false);
            return (None, true);
        };
        let choice = match self.field {
            Some((id, choice)) if id == field.id => choice,
            _ => {
                self.shift = false;
                Choice::Ours(if field.expression { Layout::Pad } else { Layout::Letters })
            }
        };
        self.field = Some((field.id, choice));
        self.shown = Some(match choice {
            Choice::System(layout) => {
                let back = back_key(field.rect, content);
                Shown { outline: back, keys: vec![(back, Key::Show(layout))], names: Vec::new(), shift: false }
            }
            Choice::Ours(layout) => {
                let names: Vec<String> = if layout == Layout::Pad { names.into_iter().take(NAMES).collect() } else { Vec::new() };
                let placed = self.placed.get(layout.place()).copied().flatten();
                let (outline, keys) = self::layout(layout, field.expression, field.rect, names.len(), content, placed);
                Shown { outline, keys, names, shift: self.shift }
            }
        });
        (self.shown.as_ref(), matches!(choice, Choice::System(_)))
    }

    /// The key to show lit at `now`, and how long until it goes out.
    pub fn lit(&self, now: f64) -> Option<(Key, f64)> {
        self.lit.and_then(|(key, at)| (now - at < LIT).then_some((key, LIT - (now - at))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENT: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(1366.0, 1000.0) };

    fn field_at(x: f32, y: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(120.0, 28.0))
    }

    fn expression(id: &str, rect: Rect) -> Option<Field> {
        Some(Field { id: Id::unique(id), rect, expression: true })
    }

    fn text(id: &str, rect: Rect) -> Option<Field> {
        Some(Field { id: Id::unique(id), rect, expression: false })
    }

    /// A point inside the first key that does `key`.
    fn at(shown: &Shown, key: Key) -> Pos2 {
        shown.keys.iter().find(|(_, k)| *k == key).map(|(rect, _)| rect.center()).unwrap_or(Pos2::ZERO)
    }

    fn press(at: Pos2) -> Event {
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }
    }

    fn release(at: Pos2) -> Event {
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }
    }

    fn tap(at: Pos2) -> Vec<Event> {
        vec![Event::PointerMoved(at), press(at), release(at)]
    }

    fn run(keys: &mut Keys, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { events, time: Some(1.0), ..Default::default() };
        keys.input(&mut raw);
        raw.events
    }

    fn shown(keys: &mut Keys, field: Option<Field>, names: &[&str]) -> Shown {
        let (shown, _) = keys.frame(field, names.iter().map(|s| (*s).to_owned()).collect(), CONTENT);
        shown.cloned().expect("keys show")
    }

    fn typed(events: &[Event]) -> String {
        events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect()
    }

    #[test]
    fn the_pad_sits_beside_the_box_inside_the_screen() {
        let field = field_at(400.0, 300.0);
        let (outline, keys) = layout(Layout::Pad, true, field, 0, CONTENT, None);
        assert_eq!(outline.left(), field.right() + BESIDE, "to the right of the box");
        assert_eq!(outline.top(), field.top(), "level with the box");
        assert!(!outline.intersects(field));
        assert!(keys.iter().all(|(rect, _)| outline.contains_rect(*rect)));
        assert!(keys.iter().all(|(rect, _)| rect.height() >= HANDLE), "every key is large enough to hit");

        // At the right edge it goes left; at the bottom it rises to stay on screen.
        let field = field_at(1200.0, 950.0);
        let (outline, _) = layout(Layout::Pad, true, field, 4, CONTENT, None);
        assert_eq!(outline.right(), field.left() - BESIDE);
        assert!(CONTENT.contains_rect(outline));
        assert!(!outline.intersects(field));

        // A box as wide as the screen: the pad goes below it, or above it near the bottom.
        let wide = Rect::from_min_size(pos2(0.0, 100.0), vec2(1366.0, 28.0));
        assert_eq!(layout(Layout::Pad, true, wide, 0, CONTENT, None).0.top(), wide.bottom() + BESIDE);
        let wide = Rect::from_min_size(pos2(0.0, 900.0), vec2(1366.0, 28.0));
        assert_eq!(layout(Layout::Pad, true, wide, 0, CONTENT, None).0.bottom(), wide.top() - BESIDE);
    }

    #[test]
    fn the_keyboard_is_compact_and_goes_below_the_box() {
        let field = field_at(400.0, 300.0);
        let (outline, keys) = layout(Layout::Letters, false, field, 6, CONTENT, None);
        assert!(outline.width() < CONTENT.width() / 2.5, "about a third of the screen: {}", outline.width());
        assert_eq!(outline.top(), field.bottom() + BESIDE);
        assert!(!keys.iter().any(|(_, k)| matches!(k, Key::Name(_))), "the names are the pad's");
        // Every row ends at the same right edge.
        let right = keys.iter().filter(|(_, k)| *k != Key::Handle).map(|(r, _)| r.right()).fold(0.0, f32::max);
        assert!((outline.right() - GAP - right).abs() < 0.01);
        for ch in 'a'..='z' {
            assert!(keys.iter().any(|(_, k)| *k == Key::Letter(ch)), "{ch} is on the keyboard");
        }
        let field = field_at(400.0, 900.0);
        assert_eq!(layout(Layout::Letters, false, field, 0, CONTENT, None).0.bottom(), field.top() - BESIDE, "above, near the bottom");
    }

    #[test]
    fn no_two_keys_overlap_and_every_key_has_its_place() {
        for (kind, expression) in [(Layout::Pad, true), (Layout::Letters, false), (Layout::Signs, false)] {
            let (outline, keys) = layout(kind, expression, field_at(100.0, 100.0), 9, CONTENT, None);
            for (i, (a, _)) in keys.iter().enumerate() {
                assert!(outline.contains_rect(*a), "{kind:?}: {a:?} outside");
                for (b, _) in &keys[i + 1..] {
                    assert!(!a.shrink(0.5).intersects(b.shrink(0.5)), "{kind:?}: {a:?} overlaps {b:?}");
                }
            }
            for key in [Key::Delete, Key::Enter, Key::Escape, Key::Full, Key::Handle] {
                assert!(keys.iter().any(|(_, k)| *k == key), "{kind:?}: {key:?}");
            }
        }
        let (_, keys) = layout(Layout::Pad, true, field_at(100.0, 100.0), 9, CONTENT, None);
        assert_eq!(keys.iter().filter(|(_, k)| matches!(k, Key::Name(_))).count(), NAMES, "at most six names");
    }

    #[test]
    fn keys_type_into_the_box_and_the_press_never_reaches_it() {
        let mut keys = Keys::default();
        let shown = shown(&mut keys, expression("box", field_at(400.0, 300.0)), &["width", "gap"]);
        let mut out = Vec::new();
        for key in [Key::Type("1"), Key::Type("2"), Key::Type("/"), Key::Name(1), Key::Delete, Key::Enter] {
            out.extend(run(&mut keys, tap(at(&shown, key))));
        }
        assert!(out.iter().all(|e| !matches!(e, Event::PointerButton { .. } | Event::PointerMoved(_))), "the box saw no press: {out:?}");
        assert_eq!(typed(&out), "12/gap");
        let pressed: Vec<EguiKey> = out.iter().filter_map(|e| if let Event::Key { key, pressed: true, .. } = e { Some(*key) } else { None }).collect();
        assert_eq!(pressed, [EguiKey::Backspace, EguiKey::Enter]);
        // A press elsewhere is the app's.
        assert_eq!(run(&mut keys, tap(pos2(10.0, 10.0))).len(), 3);
    }

    #[test]
    fn a_name_box_gets_the_keyboard_and_shift_makes_one_capital() {
        let mut keys = Keys::default();
        let field = text("name", field_at(400.0, 300.0));
        let letters = shown(&mut keys, field, &["width"]);
        assert!(letters.names.is_empty());
        let mut out = run(&mut keys, tap(at(&letters, Key::Shift)));
        let capital = shown(&mut keys, field, &[]);
        assert!(capital.shift);
        for key in [Key::Letter('b'), Key::Letter('o'), Key::Type(" "), Key::Letter('x')] {
            out.extend(run(&mut keys, tap(at(&capital, key))));
        }
        assert_eq!(typed(&out), "Bo x");

        // 123 gives the digits and signs, ABC the letters again.
        run(&mut keys, tap(at(&letters, Key::Show(Layout::Signs))));
        let signs = shown(&mut keys, field, &[]);
        assert!(signs.keys.iter().any(|(_, k)| *k == Key::Type("@")));
        run(&mut keys, tap(at(&signs, Key::Show(Layout::Letters))));
        assert_eq!(shown(&mut keys, field, &[]).keys, letters.keys);
    }

    #[test]
    fn the_pad_and_the_keyboard_reach_each_other() {
        let mut keys = Keys::default();
        let field = expression("box", field_at(400.0, 300.0));
        let pad = shown(&mut keys, field, &[]);
        run(&mut keys, tap(at(&pad, Key::Show(Layout::Letters))));
        let letters = shown(&mut keys, field, &[]);
        assert!(letters.keys.iter().any(|(_, k)| *k == Key::Letter('q')));
        // From an expression box, 123 is the pad.
        run(&mut keys, tap(at(&letters, Key::Show(Layout::Pad))));
        assert_eq!(shown(&mut keys, field, &[]).keys, pad.keys);
    }

    #[test]
    fn a_press_on_the_keys_keeps_its_release_and_its_slide() {
        let mut keys = Keys::default();
        let shown = shown(&mut keys, expression("box", field_at(400.0, 300.0)), &[]);
        let down = run(&mut keys, vec![press(at(&shown, Key::Type("5")))]);
        assert_eq!(down, vec![Event::Text("5".into())]);
        // The tip slides off and lifts there: still the keys'.
        let off = pos2(10.0, 10.0);
        assert!(run(&mut keys, vec![Event::PointerMoved(off), release(off)]).is_empty());
        // The next press elsewhere is the app's again.
        assert_eq!(run(&mut keys, tap(off)).len(), 3);
    }

    #[test]
    fn the_handle_moves_the_keys_and_they_stay_where_put() {
        let mut keys = Keys::default();
        let field = text("name", field_at(400.0, 300.0));
        let before = shown(&mut keys, field, &[]);
        let grip = at(&before, Key::Handle);
        let to = grip + vec2(-300.0, 250.0);
        let out = run(&mut keys, vec![press(grip), Event::PointerMoved(grip + vec2(-150.0, 100.0)), Event::PointerMoved(to), release(to)]);
        assert!(out.is_empty(), "the drag is the keys': {out:?}");
        let after = shown(&mut keys, field, &[]);
        assert_eq!(after.outline.min, before.outline.min + (to - grip));

        // Another box, even after the focus left: the keyboard is where the handle put it.
        keys.frame(None, Vec::new(), CONTENT);
        let other = shown(&mut keys, text("search", field_at(900.0, 100.0)), &[]);
        assert_eq!(other.outline.min, after.outline.min);
        // The pad keeps its own place, beside the box.
        let pad = shown(&mut keys, expression("box", field_at(400.0, 300.0)), &[]);
        assert_eq!(pad.outline.left(), 400.0 + 120.0 + BESIDE);

        // Dragged off the screen, it stays on it.
        let field = text("name", field_at(400.0, 300.0));
        let shown_now = shown(&mut keys, field, &[]);
        let grip = at(&shown_now, Key::Handle);
        run(&mut keys, vec![press(grip), Event::PointerMoved(pos2(-500.0, 5000.0)), release(pos2(-500.0, 5000.0))]);
        assert!(CONTENT.contains_rect(shown(&mut keys, field, &[]).outline));
    }

    #[test]
    fn full_brings_the_system_keyboard_for_that_box_and_the_key_beside_it_takes_it_away() {
        let mut keys = Keys::default();
        let field = expression("box", field_at(400.0, 300.0));
        let (_, system) = keys.frame(field, Vec::new(), CONTENT);
        assert!(!system, "ours stand in for the system's keyboard");
        let pad = shown(&mut keys, field, &[]);
        run(&mut keys, tap(at(&pad, Key::Full)));
        let (back, system) = keys.frame(field, Vec::new(), CONTENT);
        assert!(system);
        let back = back.cloned().expect("the key beside the box shows");
        assert!(!back.ours());
        assert_eq!(back.keys.first().map(|(_, k)| *k), Some(Key::Show(Layout::Pad)));
        assert!(!back.outline.intersects(field_at(400.0, 300.0)));

        // Another box gets ours again; so does the first once it has lost the focus.
        assert!(!keys.frame(text("other", field_at(400.0, 300.0)), Vec::new(), CONTENT).1);
        assert!(!keys.frame(field, Vec::new(), CONTENT).1, "the system's keyboard was for that focus only");

        run(&mut keys, tap(at(&pad, Key::Full)));
        keys.frame(field, Vec::new(), CONTENT);
        run(&mut keys, tap(back.outline.center()));
        assert!(!keys.frame(field, Vec::new(), CONTENT).1, "the key beside the box brought the pad back");
    }

    #[test]
    fn without_a_text_box_the_keys_take_nothing() {
        let mut keys = Keys::default();
        let (shown, system) = keys.frame(None, vec!["width".into()], CONTENT);
        assert!(shown.is_none());
        assert!(system);
        assert_eq!(run(&mut keys, tap(pos2(500.0, 320.0))).len(), 3);
    }

    #[test]
    fn a_pressed_key_is_lit_for_a_moment() {
        let mut keys = Keys::default();
        let shown = shown(&mut keys, expression("box", field_at(400.0, 300.0)), &[]);
        run(&mut keys, tap(at(&shown, Key::Type("7"))));
        assert_eq!(keys.lit(1.0).map(|(k, _)| k), Some(Key::Type("7")));
        assert!(keys.lit(1.0 + LIT + 0.01).is_none());
    }

    #[test]
    fn a_name_key_beyond_the_list_types_nothing() {
        assert!(Key::Name(3).events(&["a".into()], false).is_empty());
    }

    #[test]
    fn what_a_key_types_is_drawn_larger_than_what_it_does() {
        assert!(Key::Letter('a').types() && Key::Type("7").types());
        assert!(!Key::Type(" ").types() && !Key::Enter.types() && !Key::Full.types());
    }
}
