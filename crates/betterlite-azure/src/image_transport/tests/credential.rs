#![cfg(test)]

use super::support::{API_KEY, PROVIDER_BASE, Step, failure_of, reply, run};

const DIRECT_RESULT: &str = r#"{"created":1,"data":[]}"#;

fn identity_step(key: &'static str) -> Step {
    let mut step = Step::direct(PROVIDER_BASE, None, vec![reply(200, vec![], DIRECT_RESULT)]);
    step.headers = vec![("api-key", key), ("content-type", "application/json")];
    step
}

fn failing_step() -> Step {
    Step::preview(vec![reply(500, vec![], "the submit request failed")])
}

#[test]
fn azure_image_transport_credential_identity_isolation() {
    let one = run(identity_step("key-one"));
    let two = run(identity_step("key-two"));
    assert!(
        one.harness
            .calls()
            .iter()
            .all(|call| call.header("api-key") == Some("key-one"))
    );
    assert!(
        two.harness
            .calls()
            .iter()
            .all(|call| call.header("api-key") == Some("key-two"))
    );
    assert!(!one.harness.calls().is_empty());
    assert!(!two.harness.calls().is_empty());
    let debug = one.harness.request_debug();
    assert!(debug.iter().all(|line| !line.contains("key-one")));
    assert!(debug.iter().all(|line| !line.contains("key-two")));
    assert!(debug.iter().any(|line| line.contains("Sensitive")));
}

#[test]
fn azure_image_transport_secret_safe_failure() {
    let outcome = run(failing_step());
    let failure = failure_of(&outcome);
    assert!(!format!("{failure:?}").contains(API_KEY));
    assert!(!failure.to_string().contains(API_KEY));
    let debug = outcome.harness.request_debug();
    assert!(!debug.iter().any(|line| line.contains(API_KEY)));
    assert!(debug.iter().any(|line| line.contains("Sensitive")));
}
