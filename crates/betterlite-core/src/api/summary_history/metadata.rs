use betterlite_prompts::arguments::json::Object;

// the summary call's spend and limits land on the caller's key, team, user, end user, project and model budgets
pub const PROPAGATED_METADATA_KEYS: [&str; 16] = [
    "user_api_key",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_team_alias",
    "user_api_key_user_id",
    "user_api_key_user_email",
    "user_api_key_org_id",
    "user_api_key_project_id",
    "user_api_key_end_user_id",
    "user_api_end_user_max_budget",
    "user_api_key_model_max_budget",
    "user_api_key_team_model_max_budget",
    "user_api_key_user_model_max_budget",
    "user_api_key_end_user_model_max_budget",
    "litellm_call_id",
    "litellm_parent_otel_span",
];

#[must_use]
pub fn propagate_metadata(parent: Option<&Object>) -> Object {
    let mut propagated = Object::new();
    let Some(fields) = parent else {
        return propagated;
    };
    for key in PROPAGATED_METADATA_KEYS {
        if let Some(value) = fields.get(key) {
            propagated.insert(key.to_owned(), value.clone());
        }
    }
    propagated
}
