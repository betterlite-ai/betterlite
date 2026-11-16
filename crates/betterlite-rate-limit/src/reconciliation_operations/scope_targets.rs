use std::collections::BTreeSet;

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ScopeMetadata {
    pub user_api_key_hash: Option<String>,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_org_id: Option<String>,
    pub user_api_key_project_id: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
}

#[must_use]
pub fn collect_tpm_scope_targets(
    metadata: &ScopeMetadata,
    request_user: Option<&str>,
    model_group: Option<&str>,
    tpm_limited_tags: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut targets = identity_targets(metadata, request_user);
    model_scope_targets(&mut targets, metadata, model_group);
    agent_targets(&mut targets, metadata);
    for tag in tpm_limited_tags {
        targets.push(("tag".to_owned(), tag.clone()));
    }
    targets
}

fn identity_targets(metadata: &ScopeMetadata, request_user: Option<&str>) -> Vec<(String, String)> {
    let user_id = present(metadata.user_api_key_user_id.as_deref());
    let team_id = present(metadata.user_api_key_team_id.as_deref());
    let mut targets = Vec::new();
    push_target(
        &mut targets,
        "api_key",
        present(metadata.user_api_key_hash.as_deref()),
    );
    push_target(&mut targets, "user", user_id);
    push_target(&mut targets, "team", team_id);
    if let (Some(team), Some(user)) = (team_id, user_id) {
        targets.push(("team_member".to_owned(), format!("{team}:{user}")));
    }
    let end_user =
        present(request_user).or_else(|| present(metadata.user_api_key_end_user_id.as_deref()));
    push_target(&mut targets, "end_user", end_user);
    push_target(
        &mut targets,
        "organization",
        present(metadata.user_api_key_org_id.as_deref()),
    );
    targets
}

fn model_scope_targets(
    targets: &mut Vec<(String, String)>,
    metadata: &ScopeMetadata,
    model_group: Option<&str>,
) {
    let Some(group) = present(model_group) else {
        return;
    };
    for (key, scope_id) in [
        ("model_per_key", metadata.user_api_key_hash.as_deref()),
        ("model_per_team", metadata.user_api_key_team_id.as_deref()),
        (
            "model_per_organization",
            metadata.user_api_key_org_id.as_deref(),
        ),
        (
            "model_per_project",
            metadata.user_api_key_project_id.as_deref(),
        ),
    ] {
        if let Some(id) = present(scope_id) {
            targets.push((key.to_owned(), format!("{id}:{group}")));
        }
    }
}

fn agent_targets(targets: &mut Vec<(String, String)>, metadata: &ScopeMetadata) {
    let Some(agent) = present(metadata.agent_id.as_deref()) else {
        return;
    };
    targets.push(("agent".to_owned(), agent.to_owned()));
    let session_id =
        present(metadata.session_id.as_deref()).or_else(|| present(metadata.trace_id.as_deref()));
    if let Some(session) = session_id {
        targets.push(("agent_session".to_owned(), format!("{agent}:{session}")));
    }
}

fn push_target(targets: &mut Vec<(String, String)>, key: &'static str, target_id: Option<&str>) {
    if let Some(value) = target_id {
        targets.push((key.to_owned(), value.to_owned()));
    }
}

const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}
