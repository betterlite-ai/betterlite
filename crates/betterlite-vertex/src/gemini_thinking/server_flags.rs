use betterlite_prompts::arguments::json::{Json, Object};

pub fn apply_server_tool_flags(non_default_params: &Object, optional_params: &mut Object) {
    for key in [
        "include_server_side_tool_invocations",
        "includeServerSideToolInvocations",
    ] {
        if matches!(non_default_params.get(key), Some(Json::Bool(true)))
            || matches!(optional_params.get(key), Some(Json::Bool(true)))
        {
            optional_params.insert(
                "include_server_side_tool_invocations".to_owned(),
                Json::Bool(true),
            );
            return;
        }
    }
}
