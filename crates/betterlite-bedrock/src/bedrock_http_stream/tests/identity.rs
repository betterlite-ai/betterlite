use betterlite_models::decisions::Json;

use super::support::{
    BASE, Call, Consume, FIRST, Inputs, Reply, header_map, invoke, member, messages, run,
};
use crate::bedrock_http_stream::invoke::InvokeCall;

// One credential per identity, and a body that names it, so a shared response or log is visible.
type Identity = (&'static str, &'static str, &'static str);

fn identity_call<'call>(messages: &'call Json, identity: Identity) -> InvokeCall<'call> {
    let (name, authorization, data) = identity;
    let mut call = invoke(BASE, data, messages);
    call.headers = header_map(&[("authorization", authorization), ("x-identity", name)]);
    call
}

#[test]
fn bedrock_http_stream_credential_identity_isolation() {
    let messages = messages();
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let alpha = (
        "alpha",
        "Bearer fixture-alpha-credential",
        r#"{"inputText":"alpha"}"#,
    );
    let beta = (
        "beta",
        "Bearer fixture-beta-credential",
        r#"{"inputText":"beta"}"#,
    );
    let alpha_outcome = run(
        Call::Invoke(identity_call(&messages, alpha)),
        &inputs,
        Consume::None,
    );
    let beta_outcome = run(
        Call::Invoke(identity_call(&messages, beta)),
        &inputs,
        Consume::None,
    );

    let posts = inputs.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(inputs.calls(), 2);
    assert_eq!(posts[0].body, alpha.2);
    assert_eq!(posts[1].body, beta.2);
    assert_eq!(
        posts[0].headers,
        vec![
            ("authorization".to_owned(), alpha.1.to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-identity".to_owned(), alpha.0.to_owned()),
        ]
    );
    assert_eq!(
        posts[1].headers,
        vec![
            ("authorization".to_owned(), beta.1.to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-identity".to_owned(), beta.0.to_owned()),
        ]
    );
    let alpha_log = alpha_outcome.logged(0);
    let beta_log = beta_outcome.logged(0);
    assert_eq!(
        member(member(alpha_log, "additional_args"), "complete_input_dict"),
        &Json::Text(alpha.2.to_owned())
    );
    assert_eq!(
        member(member(beta_log, "additional_args"), "complete_input_dict"),
        &Json::Text(beta.2.to_owned())
    );

    // Neither the other credential nor its body rides on this identity's request or log entry.
    assert!(!posts[0].body.contains(beta.0));
    assert!(!serde_json::to_string(alpha_log).unwrap().contains(beta.1));
    assert!(!posts[1].body.contains(alpha.0));
    assert!(!serde_json::to_string(beta_log).unwrap().contains(alpha.1));

    // Asking as the first identity again builds a fresh request and log entry, not a cached answer.
    let replay = run(
        Call::Invoke(identity_call(&messages, alpha)),
        &inputs,
        Consume::None,
    );
    let replayed = inputs.posts();
    assert_eq!(replayed.len(), 3);
    assert_eq!(replayed[0], replayed[2]);
    assert_ne!(replayed[0], replayed[1]);
    assert_eq!(replay.logs.len(), 1);
    let logged = replay.logged(0);
    assert_eq!(member(logged, "input"), &messages);
    assert!(!serde_json::to_string(logged).unwrap().contains(beta.1));
}
