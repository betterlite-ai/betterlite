mod tests;

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::error::GateError;

#[derive(Serialize)]
struct Settings<'dirs> {
    sandbox: Sandbox<'dirs>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sandbox<'dirs> {
    enabled: bool,
    auto_allow_bash_if_sandboxed: bool,
    filesystem: Filesystem<'dirs>,
    network: Network,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Filesystem<'dirs> {
    allow_write: Vec<&'dirs str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Network {
    allowed_domains: [&'static str; 1],
}

pub(crate) fn write_settings(
    path: &Path,
    worktree: &str,
    writable: &[String],
) -> Result<(), GateError> {
    let mut allow_write = vec![worktree];
    allow_write.extend(writable.iter().map(String::as_str));
    let settings = Settings {
        sandbox: Sandbox {
            enabled: true,
            auto_allow_bash_if_sandboxed: true,
            filesystem: Filesystem { allow_write },
            network: Network {
                allowed_domains: ["*"],
            },
        },
    };
    let text = serde_json::to_string_pretty(&settings).map_err(|source| GateError::Json {
        what: "the Claude sandbox settings".to_owned(),
        source,
    })?;
    fs::write(path, text).map_err(|source| GateError::Write {
        path: path.to_owned(),
        source,
    })
}
