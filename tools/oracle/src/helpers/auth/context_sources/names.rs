use std::collections::BTreeMap;

use betterlite_auth::auth_context_sources::claude::{
    AliasMap, ClaudeNameSources, RouterNames, claude_code_requested_group,
};
use serde_json::{Value, json};

use super::Input;

struct NamesFixture {
    alias_names: Vec<String>,
    model_ids: Vec<String>,
    candidate_ids: BTreeMap<String, Vec<String>>,
}

impl RouterNames for NamesFixture {
    fn model_group_alias_contains(&self, name: &str) -> bool {
        self.alias_names.iter().any(|alias| alias == name)
    }

    fn has_model_id(&self, name: &str) -> bool {
        self.model_ids.iter().any(|id| id == name)
    }

    fn candidate_model_ids_for_route(&self, name: &str, _team_id: Option<&str>) -> Vec<String> {
        self.candidate_ids.get(name).cloned().unwrap_or_default()
    }
}

pub(super) fn claude_answer(input: &Input) -> Value {
    let router = NamesFixture {
        alias_names: input.router_model_group_alias.clone(),
        model_ids: input.router_model_ids.clone(),
        candidate_ids: input.candidate_ids.clone(),
    };
    let names = ClaudeNameSources {
        global_aliases: &input.global_aliases,
        key_aliases: input.key_aliases.as_ref().map(AliasMap::Json),
        team_model_aliases: input.team_model_aliases.as_ref().map(AliasMap::Json),
        hierarchical_model_group_alias: input.hierarchical_aliases.as_ref().map(AliasMap::Json),
        router: Some(&router),
        team_id: input.team_id.as_deref(),
    };
    let requested = input.requested.as_deref().unwrap_or_default();
    json!(claude_code_requested_group(requested, &names))
}
