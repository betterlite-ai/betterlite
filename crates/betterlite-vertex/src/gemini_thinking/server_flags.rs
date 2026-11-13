use betterlite_prompts::arguments::json::{Json, Object};

pub fn apply_server_tool_flags(non_default_params: &Object, optional_params: &mut Object) {
    let flagged = |params: &Object| {
        [
            "include_server_side_tool_invocations",
            "includeServerSideToolInvocations",
        ]
        .iter()
        .any(|key| matches!(params.get(*key), Some(Json::Bool(true))))
    };
    if flagged(non_default_params) || flagged(optional_params) {
        optional_params.insert(
            "include_server_side_tool_invocations".to_owned(),
            Json::Bool(true),
        );
    }
}
