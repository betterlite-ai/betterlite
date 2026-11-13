#![cfg(test)]

use std::fs;
use std::io;

use tracing::subscriber::with_default;

use super::{OperatorOutput as _, StreamOutput};

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

#[test]
fn stream_output_writes_the_instruction_line() {
    let mut output = StreamOutput { writer: Vec::new() };

    output.instructions("Please visit https://github.com/login/device and enter code ABCD.");

    assert_eq!(
        output.writer,
        b"Please visit https://github.com/login/device and enter code ABCD.\n"
    );
}

struct FailingWriter;

impl io::Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("closed"))
    }
}

#[test]
fn stream_output_logs_a_write_failure() {
    let ((), output) = logged(|| {
        let mut output = StreamOutput {
            writer: FailingWriter,
        };
        output.instructions("Please visit the device page.");
    });

    assert!(output.contains("WARN the Copilot device login instructions could not be written"));
}
