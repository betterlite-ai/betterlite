mod tests;

use std::env;
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::error::GateError;

#[derive(Serialize)]
struct Settings<'dirs> {
    sandbox: Sandbox<'dirs>,
    permissions: Permissions,
    // a config folder without the user's settings would add Claude Code's Co-Authored-By line to commits
    attribution: Attribution,
}

#[derive(Serialize)]
struct Attribution {
    commit: &'static str,
    pr: &'static str,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allow_unix_sockets: Vec<String>,
}

#[derive(Serialize)]
struct Permissions {
    deny: [String; 10],
}

pub(crate) fn write_settings(
    path: &Path,
    worktree: &str,
    home: &str,
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
                allow_unix_sockets: env::var("GATE_POSTGRES_SOCKET").ok().into_iter().collect(),
            },
        },
        // on 11-13 workers got past bin/cargo by unsetting GATE_DISPATCHED or calling cargo by path, and a local gate run or cargo fmt took minutes while CI runs them anyway
        permissions: Permissions {
            deny: [
                "Bash(env -u GATE_DISPATCHED:*)".to_owned(),
                "Bash(unset GATE_DISPATCHED:*)".to_owned(),
                "Bash(GATE_DISPATCHED=:*)".to_owned(),
                "Bash(~/.cargo/bin/cargo:*)".to_owned(),
                "Bash($HOME/.cargo/bin/cargo:*)".to_owned(),
                format!("Bash({home}/.cargo/bin/cargo:*)"),
                "Bash(cargo gate run:*)".to_owned(),
                "Bash(cargo gate preflight:*)".to_owned(),
                "Bash(cargo gate hook:*)".to_owned(),
                "Bash(cargo fmt:*)".to_owned(),
            ],
        },
        attribution: Attribution { commit: "", pr: "" },
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
