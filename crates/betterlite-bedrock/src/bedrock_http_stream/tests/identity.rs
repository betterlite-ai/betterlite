use serde_json::{json, to_value};

use super::support::{
    BASE, Call, Consume, FIRST, Inputs, Reply, header_map, invoke, messages, run,
};

// One credential per identity, and a body that names it, so a shared response or log is visible.
const IDENTITIES: [(&str, &str, &str); 2] = [
    (
        "alpha",
        "Bearer fixture-alpha-credential",
        r#"{"inputText":"alpha"}"#,
    ),
    (
        "beta",
        "Bearer fixture-beta-credential",
        r#"{"inputText":"beta"}"#,
    ),
];

#[test]
fn bedrock_http_stream_credential_identity_isolation() {
    let messages = messages();
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut outcomes = Vec::new();
    for (identity, authorization, data) in IDENTITIES {
        let mut call = invoke(BASE, data, &messages);
        call.headers = header_map(&[("authorization", authorization), ("x-identity", identity)]);
        outcomes.push(run(Call::Invoke(call), &inputs, Consume::None));
    }
    let posts = inputs.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(inputs.calls(), 2);

    for index in 0..IDENTITIES.len() {
        let (identity, authorization, data) = IDENTITIES[index];
        let post = &posts[index];
        assert_eq!(post.body, data);
        assert_eq!(
            post.headers,
            vec![
                ("authorization".to_owned(), authorization.to_owned()),
                ("content-type".to_owned(), "application/json".to_owned()),
                ("x-identity".to_owned(), identity.to_owned()),
            ]
        );
        let logged = to_value(outcomes[index].logged(0)).unwrap();
        assert_eq!(
            logged["additional_args"]["complete_input_dict"],
            json!(data)
        );
        for (other, other_authorization, _) in IDENTITIES {
            if other == identity {
                continue;
            }
            // Neither the other credential nor its body rides on this request or its log entry.
            assert!(
                !post
                    .headers
                    .iter()
                    .any(|(_, value)| value.contains(other_authorization))
            );
            assert!(!post.body.contains(other));
            assert!(!logged.to_string().contains(other_authorization));
        }
    }

    // Asking as the first identity again builds a fresh request and log entry, not a cached answer.
    let mut call = invoke(BASE, IDENTITIES[0].2, &messages);
    call.headers = header_map(&[
        ("authorization", IDENTITIES[0].1),
        ("x-identity", IDENTITIES[0].0),
    ]);
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    let posts = inputs.posts();
    assert_eq!(posts.len(), 3);
    assert_eq!(posts[0], posts[2]);
    assert_ne!(posts[0], posts[1]);
    assert_eq!(outcome.logs.len(), 1);
    let logged = to_value(outcome.logged(0)).unwrap();
    assert_eq!(logged["input"], json!([{"role": "user", "content": "hi"}]));
    assert!(!logged.to_string().contains(IDENTITIES[1].1));
}
