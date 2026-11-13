#![cfg(test)]

use std::io;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use super::super::interactive::ByteSource as _;
use super::{RawModeError, RawModeGuard, TermiosBackend};

const HISTORY_CAPACITY: usize = 8;

#[derive(Clone)]
struct FakeAttrs(u32);

struct FakeBackend {
    current: u32,
    history: SyncSender<u32>,
    bytes: Vec<u8>,
    next_byte: usize,
}

impl TermiosBackend for FakeBackend {
    type Attrs = FakeAttrs;

    fn get_attrs(&mut self) -> Result<FakeAttrs, RawModeError> {
        Ok(FakeAttrs(self.current))
    }

    fn set_attrs(&mut self, attrs: &FakeAttrs) -> Result<(), RawModeError> {
        self.current = attrs.0;
        match self.history.send(attrs.0) {
            Ok(()) | Err(_) => {}
        }
        Ok(())
    }

    fn make_raw(&self, attrs: &mut FakeAttrs) {
        attrs.0 = 1;
    }

    fn read_byte(&mut self) -> io::Result<u8> {
        let byte = self.bytes.get(self.next_byte).copied();
        self.next_byte = self.next_byte.saturating_add(1);
        byte.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    }
}

fn fake_backend(bytes: Vec<u8>) -> (FakeBackend, Receiver<u32>) {
    let (sender, receiver) = sync_channel(HISTORY_CAPACITY);
    let backend = FakeBackend {
        current: 0,
        history: sender,
        bytes,
        next_byte: 0,
    };
    (backend, receiver)
}

#[test]
fn enable_switches_to_raw_attributes() {
    let (backend, history) = fake_backend(Vec::new());

    let _guard = RawModeGuard::enable(backend).expect("enable succeeds");

    assert_eq!(history.try_iter().collect::<Vec<_>>(), vec![1]);
}

#[test]
fn drop_restores_the_original_attributes_even_after_a_read_error() {
    let (backend, history) = fake_backend(Vec::new());

    let mut guard = RawModeGuard::enable(backend).expect("enable succeeds");
    guard.next_byte().unwrap_err();
    drop(guard);

    assert_eq!(history.try_iter().collect::<Vec<_>>(), vec![1, 0]);
}

#[test]
fn read_byte_returns_bytes_from_the_backend() {
    let (backend, _history) = fake_backend(vec![b'A']);

    let mut guard = RawModeGuard::enable(backend).expect("enable succeeds");

    assert_eq!(guard.next_byte().expect("byte available"), b'A');
}
