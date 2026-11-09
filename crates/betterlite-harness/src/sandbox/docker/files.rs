use super::DockerSandbox;
use super::error::DockerSandboxError;
use super::process::decode_utf8;
use crate::errors::{CappedOperation, HarnessError};

const WRITE_SCRIPT: &str = r#"mkdir -p "$(dirname "$1")" && cat > "$1""#;

pub(super) async fn read(
    sandbox: &DockerSandbox,
    path: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, HarnessError> {
    let target = sandbox.container_path(path);
    let (code, stdout, stderr) = sandbox
        .exec_capture(
            vec!["cat".to_owned(), target.clone()],
            None,
            super::MAX_HELPER_OUTPUT_BYTES,
        )
        .await?;
    match code {
        0_i32 if stdout.len() > max_bytes => Err(HarnessError::ContentLimitExceeded {
            operation: CappedOperation::Read,
            limit: max_bytes,
        }),
        0_i32 => Ok(stdout),
        _ => Err(HarnessError::from(DockerSandboxError::Read {
            target,
            stderr: decode_utf8(&stderr).trim().to_owned(),
        })),
    }
}

pub(super) async fn write(
    sandbox: &DockerSandbox,
    path: &str,
    data: &[u8],
) -> Result<(), HarnessError> {
    let target = sandbox.container_path(path);
    let command: Vec<String> = ["sh", "-c", WRITE_SCRIPT, "sh", target.as_str()]
        .map(str::to_owned)
        .to_vec();
    let (code, _stdout, stderr) = sandbox
        .exec_capture(command, Some(data), super::MAX_HELPER_OUTPUT_BYTES)
        .await?;
    if code == 0_i32 {
        return Ok(());
    }
    let reason = decode_utf8(&stderr).trim().to_owned();
    Err(HarnessError::from(DockerSandboxError::Write {
        target,
        stderr: reason,
    }))
}
