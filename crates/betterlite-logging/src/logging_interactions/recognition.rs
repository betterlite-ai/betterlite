use crate::logging_success::recognized::is_interactions_create;

// the proxy sets create_interaction/acreate_interaction and the SDK create/acreate; polls, deletes and cancels never bill
#[must_use]
pub fn is_billable_interaction(call_type: &str, has_usage: bool) -> bool {
    if !has_usage {
        return false;
    }
    is_interactions_create(call_type)
}
