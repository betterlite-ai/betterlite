use std::sync::{Mutex, PoisonError};

use super::ports::TerminalWriter;

pub fn echo_line(writer: &Mutex<&mut TerminalWriter>, message: &str) {
    let _unused = writeln!(
        writer.lock().unwrap_or_else(PoisonError::into_inner),
        "{message}"
    );
}
