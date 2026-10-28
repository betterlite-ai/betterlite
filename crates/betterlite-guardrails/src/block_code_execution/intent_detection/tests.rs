use super::{has_execution_intent, has_no_execution_intent};

#[test]
fn respects_conflicting_phrases() {
    assert!(has_no_execution_intent(
        "please explain this code; do not run it"
    ));
    assert!(!has_no_execution_intent(
        "do not run this, but run this command"
    ));
    assert!(has_execution_intent("please run this command"));
}
