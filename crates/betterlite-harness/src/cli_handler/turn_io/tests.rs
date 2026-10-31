#![cfg(test)]

use super::{TurnCursor, TurnProgress, take_ready_line};

#[test]
fn partial_stdout_waits_for_eof() {
    let mut pending = TurnCursor {
        stdout_buf: b"partial".to_vec(),
        stderr_buf: Vec::new(),
        stdout_scratch: Vec::new(),
        stderr_scratch: Vec::new(),
        stdout_done: false,
        stderr_done: false,
    };
    assert!(take_ready_line(&mut pending).is_none());
    let mut cursor = TurnCursor {
        stdout_buf: b"partial".to_vec(),
        stderr_buf: Vec::new(),
        stdout_scratch: Vec::new(),
        stderr_scratch: Vec::new(),
        stdout_done: true,
        stderr_done: false,
    };
    assert!(matches!(
        take_ready_line(&mut cursor),
        Some(TurnProgress::Stdout(data)) if data == b"partial"
    ));
}

#[test]
fn partial_stderr_waits_for_eof() {
    let mut pending = TurnCursor {
        stdout_buf: Vec::new(),
        stderr_buf: b"partial".to_vec(),
        stdout_scratch: Vec::new(),
        stderr_scratch: Vec::new(),
        stdout_done: false,
        stderr_done: false,
    };
    assert!(take_ready_line(&mut pending).is_none());
    let mut cursor = TurnCursor {
        stdout_buf: Vec::new(),
        stderr_buf: b"partial".to_vec(),
        stdout_scratch: Vec::new(),
        stderr_scratch: Vec::new(),
        stdout_done: false,
        stderr_done: true,
    };
    assert!(matches!(
        take_ready_line(&mut cursor),
        Some(TurnProgress::Stderr(data)) if data == b"partial"
    ));
}

#[test]
fn empty_stderr_at_eof_is_not_an_event() {
    let mut cursor = TurnCursor {
        stdout_buf: Vec::new(),
        stderr_buf: Vec::new(),
        stdout_scratch: Vec::new(),
        stderr_scratch: Vec::new(),
        stdout_done: false,
        stderr_done: true,
    };
    assert!(take_ready_line(&mut cursor).is_none());
}
