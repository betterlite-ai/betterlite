use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::types::PermissionMode;

const NORMALIZED_TO_NATIVE: [(&str, &str); 4] = [
    ("read", "read_file"),
    ("write", "write_file"),
    ("edit", "edit_file"),
    ("bash", "execute"),
];
pub const WRITE_TOOLS: [&str; 3] = ["write_file", "edit_file", "delete"];
pub const EXECUTE_TOOLS: [&str; 1] = ["execute"];
const APPROVAL_TOOLS: [&str; 4] = ["write_file", "edit_file", "delete", "execute"];
pub(crate) const BUILTIN_TOOLS: [&str; 10] = [
    "ls",
    "read_file",
    "write_file",
    "edit_file",
    "delete",
    "glob",
    "grep",
    "execute",
    "write_todos",
    "task",
];

#[must_use]
pub fn native_tool_name(normalized: &str) -> &str {
    NORMALIZED_TO_NATIVE
        .iter()
        .find(|(key, _)| *key == normalized)
        .map_or(normalized, |(_, native)| *native)
}

#[must_use]
pub fn normalized_tool_name(native: &str) -> &str {
    match native {
        "read_file" => "read",
        "write_file" => "write",
        "edit_file" => "edit",
        "execute" => "bash",
        other => other,
    }
}

#[must_use]
pub fn blocked_tools(permissions: PermissionMode, disable_tools: &[String]) -> BTreeSet<String> {
    let mut blocked: BTreeSet<String> = disable_tools
        .iter()
        .map(|name| native_tool_name(name).to_owned())
        .collect();
    match permissions {
        PermissionMode::ReadOnly => {
            blocked.extend(WRITE_TOOLS.iter().map(|tool| (*tool).to_owned()));
            blocked.extend(EXECUTE_TOOLS.iter().map(|tool| (*tool).to_owned()));
        }
        PermissionMode::Edit => blocked.extend(EXECUTE_TOOLS.iter().map(|tool| (*tool).to_owned())),
        PermissionMode::Ask | PermissionMode::Full => {}
    }
    blocked
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AllowedDecision {
    Approve,
    Reject,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InterruptOnConfig {
    pub allowed_decisions: Vec<AllowedDecision>,
}

#[must_use]
pub fn interrupt_config(
    permissions: PermissionMode,
    blocked: &BTreeSet<String>,
) -> Option<BTreeMap<String, InterruptOnConfig>> {
    if permissions != PermissionMode::Ask {
        return None;
    }
    Some(
        APPROVAL_TOOLS
            .iter()
            .filter(|tool| !blocked.contains(**tool))
            .map(|tool| {
                (
                    (*tool).to_owned(),
                    InterruptOnConfig {
                        allowed_decisions: vec![AllowedDecision::Approve, AllowedDecision::Reject],
                    },
                )
            })
            .collect(),
    )
}
