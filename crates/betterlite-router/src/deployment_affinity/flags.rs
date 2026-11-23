use std::collections::BTreeSet;

use indexmap::IndexMap;

// VALID_MODEL_GROUP_AFFINITY_FLAGS, sorted so the warning text is stable across runs
pub const VALID_FLAGS: [&str; 4] = [
    "deployment_affinity",
    "encrypted_content_affinity",
    "responses_api_deployment_check",
    "session_affinity",
];

// one model group whose config entry names a flag neither affinity check acts on
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownFlags {
    pub group: String,
    pub unknown: Vec<String>,
}

impl UnknownFlags {
    // the line warn_on_unknown_model_group_affinity_flags logs
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "model_group_affinity_config: unknown flag(s) {{{}}} for model group '{}'; will be ignored. Valid flags: frozenset({{{}}})",
            braced_set(self.unknown.iter().map(String::as_str)),
            self.group,
            braced_set(VALID_FLAGS.into_iter()),
        )
    }
}

// warn_on_unknown_model_group_affinity_flags: the schema owns typo detection for both affinity consumers
#[must_use]
pub fn warn_unknown_affinity_flags(
    config: Option<&IndexMap<String, Vec<String>>>,
) -> Vec<UnknownFlags> {
    let mut warnings = Vec::new();
    for (group, flags) in config.into_iter().flatten() {
        let unknown: BTreeSet<&str> = flags
            .iter()
            .map(String::as_str)
            .filter(|flag| !VALID_FLAGS.contains(flag))
            .collect();
        if !unknown.is_empty() {
            warnings.push(UnknownFlags {
                group: group.clone(),
                unknown: unknown.into_iter().map(str::to_owned).collect(),
            });
        }
    }
    warnings
}

fn braced_set<'flag, Flags>(flags: Flags) -> String
where
    Flags: Iterator<Item = &'flag str>,
{
    flags
        .map(|flag| format!("'{flag}'"))
        .collect::<Vec<_>>()
        .join(", ")
}
