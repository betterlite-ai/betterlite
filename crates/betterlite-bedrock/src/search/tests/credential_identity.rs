use crate::search::tests::support::{
    GATEWAY_URL, OTHER_ACCESS_KEY, authorization, baseline, failing, lookup, other_resolver,
    resolver, secret, sign,
};

#[test]
fn bedrock_services_search_credential_identity_isolation() {
    assert_server_tokens_do_not_cross();
    assert_caller_and_server_identities_stay_apart();
    assert_sigv4_identities_do_not_cross();
}

fn assert_server_tokens_do_not_cross() {
    let case = baseline(GATEWAY_URL);
    let first = sign(
        &case,
        &lookup(&[("AGENTCORE_GATEWAY_TOKEN", "token-one")]),
        failing(),
    )
    .unwrap();
    let second = sign(
        &case,
        &lookup(&[("AGENTCORE_GATEWAY_TOKEN", "token-two")]),
        failing(),
    )
    .unwrap();
    assert_eq!(authorization(&first), "Bearer token-one");
    assert_eq!(authorization(&second), "Bearer token-two");
    assert!(case.headers.get("Authorization").is_none());
    assert!(case.headers.get("authorization").is_none());
}

fn assert_caller_and_server_identities_stay_apart() {
    let mut caller = baseline(GATEWAY_URL);
    caller.api_key = Some(secret("caller-token"));
    let environment = lookup(&[("AGENTCORE_GATEWAY_TOKEN", "server-token")]);
    let signed = sign(&caller, &environment, failing()).unwrap();
    assert_eq!(authorization(&signed), "Bearer caller-token");
    let server = sign(&baseline(GATEWAY_URL), &environment, failing()).unwrap();
    assert_eq!(authorization(&server), "Bearer server-token");
}

fn assert_sigv4_identities_do_not_cross() {
    let case = baseline(GATEWAY_URL);
    let first = sign(&case, &lookup(&[]), resolver()).unwrap();
    let second = sign(&case, &lookup(&[]), other_resolver()).unwrap();
    let third = sign(&case, &lookup(&[]), resolver()).unwrap();
    assert!(authorization(&first).contains("Credential=AKIDEXAMPLE/"));
    assert!(authorization(&second).contains(OTHER_ACCESS_KEY));
    assert_eq!(authorization(&first), authorization(&third));
    assert_ne!(authorization(&first), authorization(&second));
}
