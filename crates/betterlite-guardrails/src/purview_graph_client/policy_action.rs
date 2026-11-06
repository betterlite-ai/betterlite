use crate::purview_graph_client::wire::ProcessContentResponse;

// A policy action blocks when it is a restrictAccess action whose restrictionAction is block.
#[must_use]
pub fn should_block(response: &ProcessContentResponse) -> bool {
    response.policy_actions.iter().any(|action| {
        let restricts =
            action.odata_type.contains("restrictAccessAction") || action.action == "restrictAccess";
        restricts && action.restriction_action == "block"
    })
}

mod tests;
