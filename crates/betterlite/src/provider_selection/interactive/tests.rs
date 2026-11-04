#![cfg(test)]

use std::collections::{BTreeSet, VecDeque};
use std::io;

use super::{ByteSource, InteractiveError, Key, KeySource, TtyKeySource, select_interactive};

struct ScriptedKeySource {
    keys: VecDeque<Key>,
}

impl ScriptedKeySource {
    fn new(keys: &[Key]) -> Self {
        Self {
            keys: keys.iter().copied().collect(),
        }
    }
}

impl KeySource for ScriptedKeySource {
    fn next_key(&mut self) -> io::Result<Key> {
        self.keys
            .pop_front()
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    }
}

struct FakeByteSource {
    bytes: Vec<u8>,
    next: usize,
}

impl ByteSource for FakeByteSource {
    fn next_byte(&mut self) -> io::Result<u8> {
        let byte = self.bytes.get(self.next).copied();
        self.next = self.next.saturating_add(1);
        byte.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    }
}

#[test]
fn up_arrow_from_the_first_item_wraps_to_the_last() {
    let mut keys = ScriptedKeySource::new(&[Key::Up, Key::Space, Key::Enter]);
    let mut writer = Vec::new();

    let selected = select_interactive(&mut keys, &mut writer, false).expect("selection succeeds");

    assert_eq!(selected, BTreeSet::from([5]));
}

#[test]
fn down_arrow_wraps_back_to_the_first_item_after_the_last() {
    let mut keys = ScriptedKeySource::new(&[Key::Up, Key::Down, Key::Space, Key::Enter]);
    let mut writer = Vec::new();

    let selected = select_interactive(&mut keys, &mut writer, false).expect("selection succeeds");

    assert_eq!(selected, BTreeSet::from([0]));
}

#[test]
fn space_toggles_a_provider_on_and_off() {
    let mut keys = ScriptedKeySource::new(&[Key::Space, Key::Space, Key::Space, Key::Enter]);
    let mut writer = Vec::new();

    let selected = select_interactive(&mut keys, &mut writer, false).expect("selection succeeds");

    assert_eq!(selected, BTreeSet::from([0]));
}

#[test]
fn enter_with_no_selection_chooses_the_cursor_item() {
    let mut keys = ScriptedKeySource::new(&[Key::Down, Key::Down, Key::Enter]);
    let mut writer = Vec::new();

    let selected = select_interactive(&mut keys, &mut writer, false).expect("selection succeeds");

    assert_eq!(selected, BTreeSet::from([2]));
}

#[test]
fn ctrl_c_cancels_the_selection() {
    let mut keys = ScriptedKeySource::new(&[Key::Cancel]);
    let mut writer = Vec::new();

    let result = select_interactive(&mut keys, &mut writer, false);

    assert!(matches!(result, Err(InteractiveError::Cancelled)));
}

#[test]
fn ctrl_c_and_ctrl_d_bytes_both_decode_to_the_cancel_key() {
    let mut ctrl_c = TtyKeySource {
        bytes: FakeByteSource {
            bytes: vec![0x03],
            next: 0,
        },
    };
    let mut ctrl_d = TtyKeySource {
        bytes: FakeByteSource {
            bytes: vec![0x04],
            next: 0,
        },
    };

    assert_eq!(ctrl_c.next_key().expect("byte available"), Key::Cancel);
    assert_eq!(ctrl_d.next_key().expect("byte available"), Key::Cancel);
}

#[test]
fn arrow_escape_sequences_decode_to_up_and_down() {
    let mut up = TtyKeySource {
        bytes: FakeByteSource {
            bytes: vec![0x1b, b'[', b'A'],
            next: 0,
        },
    };
    let mut down = TtyKeySource {
        bytes: FakeByteSource {
            bytes: vec![0x1b, b'[', b'B'],
            next: 0,
        },
    };

    assert_eq!(up.next_key().expect("byte available"), Key::Up);
    assert_eq!(down.next_key().expect("byte available"), Key::Down);
}

#[test]
fn hides_and_shows_the_cursor_only_when_color_is_supported() {
    let mut keys = ScriptedKeySource::new(&[Key::Enter]);
    let mut writer = Vec::new();

    select_interactive(&mut keys, &mut writer, true).expect("selection succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert!(output.starts_with("\x1b[?25l"));
    assert!(output.ends_with("\x1b[?25h"));
}

#[test]
fn does_not_touch_cursor_visibility_when_color_is_unsupported() {
    let mut keys = ScriptedKeySource::new(&[Key::Enter]);
    let mut writer = Vec::new();

    select_interactive(&mut keys, &mut writer, false).expect("selection succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert!(!output.contains("\x1b[?25l"));
    assert!(!output.contains("\x1b[?25h"));
}
