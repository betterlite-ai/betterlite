use crate::purview_graph_client::wire::ProcessContentResponse;

// A policy action blocks when it is a restrictAccess action whose restrictionAction is block.
#[must_use]
pub fn should_block(response: &ProcessContentResponse) -> bool {
    response.policy_actions.iter().flatten().any(|action| {
        let restricts = action
            .odata_type
            .as_deref()
            .is_some_and(|odata_type| odata_type.contains("restrictAccessAction"))
            || action.action.as_deref() == Some("restrictAccess");
        restricts && action.restriction_action.as_deref() == Some("block")
    })
}

mod tests;
