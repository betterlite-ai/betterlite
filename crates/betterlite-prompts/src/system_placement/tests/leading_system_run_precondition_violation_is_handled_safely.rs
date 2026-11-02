use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    // A run starting at index 0 violates place_mid_conversation_system's precondition that the caller already split off the leading system run; betterlite drops the stray run rather than wrapping around like Python's negative indexing would.
    let leaked_leading_run = vec![text_message("system", "leaked"), text_message("user", "hi")];
    let placed = place_mid_conversation_system(&leaked_leading_run, true);
    assert_eq!(placed, vec![text_message("user", "hi")]);
}
