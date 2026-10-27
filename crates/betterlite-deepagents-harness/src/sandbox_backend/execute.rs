use betterlite_harness::sandbox::Sandbox;

use crate::sandbox_backend::protocol::ExecuteResponse;
use crate::sandbox_backend::{EXIT_TIMEOUT, MAX_OUTPUT_BYTES, NO_EXECUTE_ERROR, SandboxBackend};

pub(crate) async fn execute<S>(
    backend: &SandboxBackend<S>,
    command: &str,
    timeout_seconds: u64,
) -> ExecuteResponse
where
    S: Sandbox,
{
    if !backend.permissions.allow_execute {
        return ExecuteResponse {
            output: NO_EXECUTE_ERROR.to_owned(),
            exit_code: 1,
            truncated: false,
        };
    }
    let done = match backend.run(&["sh", "-c", command], timeout_seconds).await {
        Ok(done) => done,
        Err(error) => {
            return ExecuteResponse {
                output: format!("Error: {error}"),
                exit_code: EXIT_TIMEOUT,
                truncated: false,
            };
        }
    };
    let mut output = match (done.stdout.is_empty(), done.stderr.is_empty()) {
        (_, true) => done.stdout,
        (true, false) => done.stderr,
        (false, false) => format!("{}\n{}", done.stdout, done.stderr),
    };
    let truncated = output.len() > MAX_OUTPUT_BYTES;
    output.truncate(output.floor_char_boundary(MAX_OUTPUT_BYTES));
    ExecuteResponse {
        output,
        exit_code: done.exit_code,
        truncated,
    }
}
