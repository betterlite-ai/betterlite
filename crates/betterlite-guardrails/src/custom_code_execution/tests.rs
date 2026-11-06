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
