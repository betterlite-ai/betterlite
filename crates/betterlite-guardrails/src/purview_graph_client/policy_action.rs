use betterlite_prompts::arguments::json::{Json, Object};

// A policy action blocks when it is a restrictAccess action whose restrictionAction is block.
#[must_use]
pub fn should_block(response: &Object) -> bool {
    let Some(Json::List(actions)) = response.get("policyActions") else {
        return false;
    };
    actions.iter().filter_map(Json::as_object).any(|action| {
        let odata_type = action
            .get("@odata.type")
            .and_then(Json::as_str)
            .unwrap_or_default();
        let action_field = action
            .get("action")
            .and_then(Json::as_str)
            .unwrap_or_default();
        let restricts =
            odata_type.contains("restrictAccessAction") || action_field == "restrictAccess";
        restricts && action.get("restrictionAction").and_then(Json::as_str) == Some("block")
    })
}

mod tests;
