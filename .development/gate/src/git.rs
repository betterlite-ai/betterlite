use std::path::Path;
use std::process::{Command, Output};

use crate::error::GateError;

pub(crate) fn git(directory: &Path, args: &[&str]) -> Result<String, GateError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .map_err(|source| GateError::Spawn {
            program: "git".to_owned(),
            source,
        })?;
    let Output {
        status,
        stdout,
        stderr,
    } = output;
    if !status.success() {
        return Err(GateError::Git {
            args: args.join(" "),
            stderr: String::from_utf8(stderr)
                .unwrap_or_default()
                .trim()
                .to_owned(),
        });
    }
    String::from_utf8(stdout).map_err(|source| GateError::Utf8 {
        what: format!("git {}", args.join(" ")),
        source,
    })
}
