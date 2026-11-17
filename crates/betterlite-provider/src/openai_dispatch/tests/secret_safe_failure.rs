use secrecy::SecretString;

use super::support;
use crate::openai_dispatch::call::ChatCall;
use crate::openai_dispatch::compose::{chat_plan, resolve};
use crate::openai_dispatch::fixtures::answer::RecordedRequest;

const KEY: &str = "sk-fixture-super-secret";

fn call(api_base: &str) -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::chat_request(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        ),
        credentials: support::credentials(Some(api_base), Some(KEY), Some("org-fixture")),
        headers: None,
        extra_headers: None,
        timeout: None,
        metadata: None,
    }
}

// A prepared call prints its headers with the credential marked sensitive, and an owned failure names no secret.
#[test]
fn openai_provider_secret_safe_failure() {
    let mut composition = support::composition();
    composition.globals.api_key = Some(SecretString::from(KEY));
    let plan = chat_plan(&composition, &call("https://explicit.test/v1")).expect("the chat plan");
    let debug = format!("{:?}", plan.prepared);
    assert!(!debug.contains(KEY));
    assert!(!debug.contains("Bearer sk-fixture"));
    // the wire value is still the resolved credential
    let sent = RecordedRequest::of(&plan.prepared).expect("a prepared request");
    assert_eq!(
        sent.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );

    // a deployment failure names the field, never the key that was resolved beside it
    let Err(error) = chat_plan(&composition, &call("not a url")) else {
        panic!("the invalid api base");
    };
    assert!(!error.to_string().contains(KEY));
    assert!(!format!("{error:?}").contains(KEY));

    // the resolved identity's Debug is redacted too
    let resolved = resolve(
        &composition,
        &support::credentials(None, Some(KEY), Some("org-fixture")),
    );
    assert!(!format!("{resolved:?}").contains(KEY));
}
