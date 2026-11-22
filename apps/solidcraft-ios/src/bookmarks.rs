//! The files chosen in iPadOS's document picker, kept as bookmarks between runs.
//!
//! iPadOS gives the app a file the user picked (in iCloud Drive, On My iPad, another app's folder)
//! only until the app quits, and the app's own container can move when it is reinstalled. A recent
//! design's path is therefore useless at the next start; its bookmark is what brings the file back
//! (`documents::restore`). This is the store of those bookmarks, path to bytes, written as JSON in
//! the app's data folder. Pure, so its tests run anywhere.

use serde::{Deserialize, Serialize};

/// The most bookmarks kept: the recent list is shorter, so older ones are never asked for.
pub const MAX: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Entry {
    path: String,
    /// The bookmark's bytes as lower-case hex.
    bookmark: String,
}

/// Bookmarks by path, the most recent last.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Store {
    entries: Vec<Entry>,
}

impl Store {
    /// The store written in `text`; anything unreadable is an empty store (a lost bookmark only
    /// means the user picks that file again).
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"entries\":[]}".into())
    }

    /// Remember `bookmark` for `path`, as the most recent; the oldest go beyond [`MAX`].
    pub fn put(&mut self, path: &str, bookmark: &[u8]) {
        self.entries.retain(|e| e.path != path);
        self.entries.push(Entry { path: path.to_string(), bookmark: hex(bookmark) });
        let over = self.entries.len().saturating_sub(MAX);
        self.entries.drain(..over);
    }

    /// The bookmark for `path`, if one was kept and reads back.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.entries.iter().rev().find(|e| e.path == path).and_then(|e| unhex(&e.bookmark))
    }

    pub fn contains(&self, path: &str) -> bool {
        self.get(path).is_some()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bookmark_reads_back_after_a_round_trip_through_json() {
        let mut s = Store::default();
        s.put("/private/var/mobile/Library/Mobile Documents/com~apple~CloudDocs/CAD/hood.sc", &[0, 1, 0xab, 0xff]);
        let back = Store::parse(&s.to_json());
        assert_eq!(back.get("/private/var/mobile/Library/Mobile Documents/com~apple~CloudDocs/CAD/hood.sc"), Some(vec![0, 1, 0xab, 0xff]));
        assert!(!back.contains("/elsewhere.sc"));
    }

    #[test]
    fn the_same_path_again_replaces_its_bookmark_and_becomes_the_newest() {
        let mut s = Store::default();
        s.put("/a.sc", &[1]);
        s.put("/b.sc", &[2]);
        s.put("/a.sc", &[3]);
        assert_eq!(s.get("/a.sc"), Some(vec![3]));
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries.last().map(|e| e.path.as_str()), Some("/a.sc"));
    }

    #[test]
    fn only_the_newest_bookmarks_are_kept() {
        let mut s = Store::default();
        for i in 0..MAX + 5 {
            s.put(&format!("/{i}.sc"), &[1]);
        }
        assert_eq!(s.entries.len(), MAX);
        assert!(!s.contains("/0.sc") && !s.contains("/4.sc"));
        assert!(s.contains("/5.sc") && s.contains(&format!("/{}.sc", MAX + 4)));
    }

    #[test]
    fn hostile_store_text_is_an_empty_store_and_never_panics() {
        for text in [
            "",
            "null",
            "[]",
            "{\"entries\":7}",
            "{\"entries\":[{\"path\":\"/x\",\"bookmark\":\"zz\"}]}",
            "{\"entries\":[{\"path\":\"/x\",\"bookmark\":\"abc\"}]}",
            "{\"entries\":[{\"path\":\"/x\",\"bookmark\":\"éé\"}]}",
        ] {
            let s = Store::parse(text);
            assert!(!s.contains("/x"), "{text}");
        }
    }
}
