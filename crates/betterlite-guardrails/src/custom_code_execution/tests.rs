#![cfg(test)]

use std::time::Duration;

use super::sandbox::DEFAULT_FILENAME;
use super::sandbox::compile_sandboxed;

mod budgeted_iter;
mod deadline_context;
mod execution_eventloop_bounded;
mod execution_offloop_bounded;
mod execution_sandbox_exit;
mod execution_sync_bounded;
mod execution_timeout_error;
mod inplacevar;
mod interrupt_gate;
mod sandbox_async;
mod sandbox_compilation;
mod sandbox_for_loop;
mod sandbox_globals;
mod sandbox_while_loop;

const SHORT: Duration = Duration::from_millis(30);
const LONG: Duration = Duration::from_secs(10);

// Every source error names the file, the line and the reason; a grammar failure has neither.
pub(crate) fn failure(error: &super::sandbox::error::CompileError) -> (&str, u32, &str) {
    match error {
        super::sandbox::error::CompileError::Syntax {
            filename,
            line,
            message,
        }
        | super::sandbox::error::CompileError::Forbidden {
            filename,
            line,
            message,
        } => (filename, *line, message),
        super::sandbox::error::CompileError::Grammar { .. } => {
            panic!("expected a source error, got a grammar failure")
        }
    }
}

fn compiled(source: &str) -> super::sandbox::CompiledProgram {
    match compile_sandboxed(source, DEFAULT_FILENAME) {
        Ok(program) => program,
        Err(error) => panic!("expected the source to compile: {error}"),
    }
}

use std::io::{self, Write};
use std::sync::mpsc::SyncSender;
use std::sync::{Mutex, Once};

use tracing::subscriber::set_global_default;

use tracing_subscriber::fmt::MakeWriter;

// A process-wide capture so a test can see the stuck-thread log an interrupt task writes from its own thread.
static LOGS: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();

struct Sink;

struct SinkWriter;

impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(mut logs) = LOGS.lock() {
            logs.extend_from_slice(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'source> MakeWriter<'source> for Sink {
    type Writer = SinkWriter;

    fn make_writer(&'source self) -> SinkWriter {
        SinkWriter
    }
}

pub(crate) fn captured_logs() -> String {
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::ERROR)
            .with_writer(Sink)
            .finish();
        let _installed = set_global_default(subscriber);
    });
    LOGS.lock().map_or_else(
        |_| String::new(),
        |logs| String::from_utf8(logs.clone()).unwrap_or_default(),
    )
}

// Signals after budget_ok reports a spent budget, so a test can prove the worker saw its deadline.
pub(crate) fn budget_worker(
    sender: &SyncSender<()>,
) -> Result<(), super::execution::error::SandboxError<&'static str>> {
    loop {
        if super::deadline::budget_ok().is_ok() {
            continue;
        }
        let sent = sender.send(());
        return match sent {
            Ok(()) => Ok(()),
            Err(_stopped) => Err(super::execution::error::SandboxError::Failed(
                "the test stopped listening",
            )),
        };
    }
}
