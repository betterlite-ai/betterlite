use std::collections::BTreeMap;

use crate::payloads::native_tool_names;
use crate::types::PermissionMode;

// later keys win in opencode, so disable_tools denies go last
pub(super) fn permission_rules(
    permissions: PermissionMode,
    disable_tools: &[String],
) -> BTreeMap<String, String> {
    let mode_rules: &[(&str, &str)] = match permissions {
        PermissionMode::ReadOnly => &[("edit", "deny"), ("bash", "deny"), ("webfetch", "deny")],
        PermissionMode::Edit => &[("edit", "allow"), ("bash", "deny"), ("webfetch", "allow")],
        // Ask is never reached: OpenCodeHarnessConfig's capabilities() omits it
        PermissionMode::Full | PermissionMode::Ask => &[("*", "allow")],
    };
    // opencode gates write, edit and apply_patch with the single `edit` permission (NORMALIZED_TO_NATIVE)
    let mapping = BTreeMap::from([
        ("read".to_owned(), vec!["read".to_owned()]),
        ("write".to_owned(), vec!["edit".to_owned()]),
        ("edit".to_owned(), vec!["edit".to_owned()]),
        ("bash".to_owned(), vec!["bash".to_owned()]),
        ("glob".to_owned(), vec!["glob".to_owned()]),
        ("grep".to_owned(), vec!["grep".to_owned()]),
        ("ls".to_owned(), vec!["list".to_owned()]),
        (
            "web_search".to_owned(),
            vec!["webfetch".to_owned(), "websearch".to_owned()],
        ),
    ]);
    let denied = native_tool_names(disable_tools, &mapping);
    let mut rules = BTreeMap::new();
    for (key, value) in mode_rules {
        if !denied.contains(&(*key).to_owned()) {
            rules.insert((*key).to_owned(), (*value).to_owned());
        }
    }
    for native in &denied {
        rules.insert(native.clone(), "deny".to_owned());
    }
    rules
}

pub(super) fn normalize_tool_name(native: &str) -> String {
    match native {
        "edit" | "multiedit" | "patch" | "apply_patch" => "edit".to_owned(),
        "list" => "ls".to_owned(),
        "webfetch" | "websearch" => "web_search".to_owned(),
        other => other.to_owned(),
    }
}
