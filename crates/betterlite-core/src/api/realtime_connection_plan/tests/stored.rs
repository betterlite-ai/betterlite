#![cfg(test)]

use super::*;

#[tokio::test]
async fn realtime_probe_stored_precedence() {
    let mut harness = Harness::new();
    harness.stored = credentials(&[(
        "c",
        &[
            ("api_key", text("stored-key")),
            ("api_base", text("https://stored.test")),
            ("api_version", text("stored-version")),
        ],
    )]);
    let mut merged = probe_request(
        "azure",
        MODEL,
        params(&[("litellm_credential_name", text("c"))]),
    );
    merged.realtime_protocol = Some(String::from("beta"));
    harness.run(&merged, &settings()).await.unwrap();
    let connection = harness.dialer.last();
    assert_eq!(
        connection.url,
        "wss://stored.test/openai/realtime?api-version=stored-version&deployment=realtime-a"
    );
    assert_eq!(
        connection.headers,
        Headers::from([(String::from("api-key"), String::from("stored-key"))])
    );
    assert!(
        harness.azure.calls().is_empty(),
        "a truthy key skips Azure AD"
    );
    assert_eq!(
        harness.dialer.count(),
        1,
        "the probe opens exactly one handshake and sends no frames"
    );
}

#[tokio::test]
async fn realtime_probe_explicit_parameters_override_stored_values() {
    let mut harness = Harness::new();
    harness.stored = credentials(&[(
        "c",
        &[
            ("api_key", text("stored-key")),
            ("api_base", text("https://stored.test")),
            ("api_version", text("stored-version")),
        ],
    )]);
    let mut parameters = probe_request(
        "azure",
        MODEL,
        params(&[
            ("litellm_credential_name", text("c")),
            ("api_key", text("param-key")),
            ("api_base", text("https://param.test")),
            ("api_version", text("param-version")),
        ]),
    );
    parameters.realtime_protocol = Some(String::from("beta"));
    harness.run(&parameters, &settings()).await.unwrap();
    let connection = harness.dialer.last();
    assert_eq!(
        connection.url,
        "wss://param.test/openai/realtime?api-version=param-version&deployment=realtime-a"
    );
    assert_eq!(
        connection.headers,
        Headers::from([(String::from("api-key"), String::from("param-key"))])
    );
}

#[tokio::test]
async fn realtime_probe_named_arguments_override_stored_values() {
    let mut harness = Harness::new();
    harness.stored = credentials(&[(
        "c",
        &[
            ("api_key", text("stored-key")),
            ("api_base", text("https://stored.test")),
            ("api_version", text("stored-version")),
        ],
    )]);
    let mut named = probe_request(
        "azure",
        MODEL,
        params(&[("litellm_credential_name", text("c"))]),
    );
    named.api_key = Some(SecretString::from("named-key"));
    named.api_base = Some(String::from("https://named.test"));
    named.api_version = Some(String::from("named-version"));
    named.realtime_protocol = Some(String::from("beta"));
    harness.run(&named, &settings()).await.unwrap();
    let connection = harness.dialer.last();
    assert_eq!(
        connection.url,
        "wss://named.test/openai/realtime?api-version=named-version&deployment=realtime-a"
    );
    assert_eq!(
        connection.headers,
        Headers::from([(String::from("api-key"), String::from("named-key"))])
    );
}

#[tokio::test]
async fn realtime_probe_explicit_none_erases_a_stored_value() {
    let mut harness = Harness {
        azure: ScriptedAzure::with_token("ad-token"),
        ..Harness::new()
    };
    harness.stored = credentials(&[(
        "c",
        &[
            ("api_key", text("stored-key")),
            ("api_base", text("https://stored.test")),
            ("api_version", text("stored-version")),
        ],
    )]);
    let mut request = probe_request(
        "azure",
        MODEL,
        params(&[
            ("litellm_credential_name", text("c")),
            ("api_key", Json::Null),
            ("api_base", Json::Null),
            ("api_version", Json::Null),
        ]),
    );
    request.realtime_protocol = Some(String::from("beta"));
    harness.run(&request, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a",
        "an explicit None erases the stored value and the defaults apply"
    );
    let calls = harness.azure.calls();
    assert_eq!(calls.len(), 1, "an absent key resolves an Azure AD token");
    assert_eq!(calls[0].get("api_key"), Some(&Json::Null));
}

#[tokio::test]
async fn realtime_probe_credential_name_lookup_rules() {
    let mut harness = Harness::new();
    harness.stored = credentials(&[
        ("", &[("api_base", text("https://empty-name.test"))]),
        ("7", &[("api_base", text("https://numbered.test"))]),
    ]);
    let empty_name = probe_request(
        "openai",
        MODEL,
        params(&[("litellm_credential_name", text(""))]),
    );
    harness.run(&empty_name, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "wss://empty-name.test/v1/realtime?model=realtime-a",
        "an empty string still reaches the credential accessor"
    );
    let non_string = probe_request(
        "openai",
        MODEL,
        params(&[(
            "litellm_credential_name",
            Json::Number(serde_json::Number::from(7_i64)),
        )]),
    );
    harness.run(&non_string, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "wss://api.openai.com/v1/realtime?model=realtime-a",
        "a non-string credential name performs no lookup"
    );
}

#[tokio::test]
async fn realtime_probe_openai_auth_headers() {
    let harness = Harness::new();
    let absent = probe_request("openai", MODEL, JsonObject::new());
    harness.run(&absent, &settings()).await.unwrap();
    assert!(
        harness.dialer.last().headers.is_empty(),
        "an absent key sends no auth header"
    );
    let empty = probe_request("openai", MODEL, params(&[("api_key", text(""))]));
    harness.run(&empty, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().headers,
        Headers::from([(String::from("Authorization"), String::from("Bearer "))]),
        "an empty key still sends a Bearer header"
    );
}
