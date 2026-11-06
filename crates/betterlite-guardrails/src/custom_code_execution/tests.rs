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

fn compiled(source: &str) -> super::sandbox::CompiledProgram {
    match compile_sandboxed(source, DEFAULT_FILENAME) {
        Ok(program) => program,
        Err(error) => panic!("expected the source to compile: {error}"),
    }
}
