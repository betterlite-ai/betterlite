use super::*;

#[test]
fn discover_rejects_an_issuer_on_another_path() {
    let mut value = contract();
    value.issuer = "https://example.test/other".to_owned();
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&value).expect("json"),
    };
    assert!(discover_cli_auth("https://example.test", &ScriptedHttp::new(response)).is_err());
}

#[test]
fn discover_accepts_a_proxy_mounted_under_a_path_only_at_that_path() {
    let mut value = contract();
    value.issuer = "https://example.test/litellm".to_owned();
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&value).expect("json"),
    };
    let client = ScriptedHttp::new(response.clone());
    assert!(discover_cli_auth("https://example.test/litellm/", &client).is_ok());
    assert_eq!(
        client.requests()[0].url,
        "https://example.test/litellm/.well-known/litellm-cli-auth"
    );
    assert!(discover_cli_auth("https://example.test", &ScriptedHttp::new(response)).is_err());
}
