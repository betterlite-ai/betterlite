use super::DockerSandbox;
use super::error::DockerSandboxError;
use super::process::decode_utf8;
use crate::errors::HarnessError;

const WHICH_SCRIPT: &str = r#"command -v "$1""#;

pub(super) async fn which(
    sandbox: &DockerSandbox,
    binary: &str,
) -> Result<Option<String>, HarnessError> {
    let command = vec![
        "sh".to_owned(),
        "-lc".to_owned(),
        WHICH_SCRIPT.to_owned(),
        "sh".to_owned(),
        binary.to_owned(),
    ];
    let (code, stdout, _stderr) = sandbox
        .exec_capture(command, None, super::MAX_HELPER_OUTPUT_BYTES)
        .await?;
    let output = decode_utf8(&stdout);
    match (code, output.trim()) {
        (0_i32, found) if !found.is_empty() => Ok(Some(found.to_owned())),
        _ => Ok(None),
    }
}

pub(super) async fn tempdir(sandbox: &DockerSandbox) -> Result<String, HarnessError> {
    let (code, stdout, stderr) = sandbox
        .exec_capture(
            vec!["mktemp".to_owned(), "-d".to_owned()],
            None,
            super::MAX_HELPER_OUTPUT_BYTES,
        )
        .await?;
    let output = decode_utf8(&stdout);
    let path = output.trim();
    if code == 0_i32 && !path.is_empty() {
        return Ok(path.to_owned());
    }
    Err(HarnessError::from(DockerSandboxError::Tempdir {
        stderr: decode_utf8(&stderr).trim().to_owned(),
    }))
}
