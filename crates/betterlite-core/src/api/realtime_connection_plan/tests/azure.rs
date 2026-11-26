#![cfg(test)]

use super::*;

fn transcription_harness() -> Harness {
    Harness {
        metadata: StaticMetadata::new().with_transcription(MODEL),
        azure: ScriptedAzure::with_token("ad-token"),
        ..Harness::new()
    }
}

fn beta_environment() -> RealtimeProbeSettings {
    let mut environment = settings();
    environment.azure_realtime_protocol = Some(String::from("beta"));
    environment
}

// _realtime_health_check's Azure leg raises when neither key nor AD token resolves
#[tokio::test]
async fn realtime_probe_azure_configuration_errors_propagate() {
    let harness = Harness::new();
    let request = probe_request("azure", MODEL, JsonObject::new());
    let error = harness
        .run(&request, &settings())
        .await
        .expect_err("an Azure probe without a credential is a configuration error");
    assert_eq!(
        error.to_string(),
        "Missing Azure credentials for the realtime endpoint. Set an api_key, or configure Azure AD auth (azure_ad_token, tenant_id/client_id/client_secret, or a managed identity)"
    );
    assert_eq!(
        harness.dialer.count(),
        0,
        "the build fails before any connect"
    );
}

#[tokio::test]
async fn realtime_probe_azure_resolves_ad_from_the_merged_parameters() {
    let mut harness = Harness {
        azure: ScriptedAzure::with_token("resolved-ad-token"),
        ..Harness::new()
    };
    harness.stored = credentials(&[(
        "c",
        &[
            ("litellm_credential_name", text("c")),
            ("azure_ad_token", text("stored-ad-token")),
        ],
    )]);
    let empty_key = probe_request(
        "azure",
        MODEL,
        params(&[
            ("litellm_credential_name", text("c")),
            ("api_key", text("")),
        ]),
    );
    harness.run(&empty_key, &settings()).await.unwrap();
    let calls = harness.azure.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].get("azure_ad_token"),
        Some(&text("stored-ad-token")),
        "the accessor reads the merged map"
    );
    assert_eq!(
        harness.dialer.last().headers,
        Headers::from([(
            String::from("Authorization"),
            String::from("Bearer resolved-ad-token")
        )])
    );
}

#[tokio::test]
async fn realtime_probe_azure_protocol() {
    let harness = Harness {
        metadata: StaticMetadata::new()
            .with_transcription(MODEL)
            .with_endpoints("endpoint-model", &["/v1/realtime/transcription_sessions"])
            .with_endpoints("other-endpoint", &["/v1/chat/completions"]),
        ..transcription_harness()
    };
    let model = probe_request("azure", MODEL, JsonObject::new());
    harness.run(&model, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a transcription model supplies only intent=transcription with the default base"
    );
    let endpoint = probe_request("azure", "endpoint-model", JsonObject::new());
    harness.run(&endpoint, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?intent=transcription"
    );
    let other = probe_request("azure", "other-endpoint", JsonObject::new());
    harness.run(&other, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?model=other-endpoint"
    );
}

#[tokio::test]
async fn realtime_probe_azure_failing_metadata_is_not_transcription() {
    let harness = Harness {
        metadata: StaticMetadata::new().failing("failing-model"),
        ..transcription_harness()
    };
    let failing = probe_request("azure", "failing-model", JsonObject::new());
    harness.run(&failing, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?model=failing-model",
        "a failing metadata lookup is not a transcription model"
    );
}

#[tokio::test]
async fn realtime_probe_azure_protocol_precedence() {
    let harness = transcription_harness();
    let mut named = probe_request("azure", MODEL, params(&[("realtime_protocol", text("GA"))]));
    named.realtime_protocol = Some(String::from("beta"));
    let mut ga_environment = settings();
    ga_environment.azure_realtime_protocol = Some(String::from("GA"));
    harness.run(&named, &ga_environment).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "the named protocol wins over the model parameters"
    );
    let model_ga = probe_request("azure", MODEL, params(&[("realtime_protocol", text("GA"))]));
    harness.run(&model_ga, &beta_environment()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "the model protocol wins over the environment"
    );
}

#[tokio::test]
async fn realtime_probe_azure_environment_protocol() {
    let harness = transcription_harness();
    let plain = probe_request("azure", MODEL, JsonObject::new());
    harness.run(&plain, &beta_environment()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "the environment supplies beta when nothing else is configured"
    );
    harness.run(&plain, &settings()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "no configured string gives GA"
    );
}

#[tokio::test]
async fn realtime_probe_azure_truthy_non_string_protocol_falls_back_to_ga() {
    let harness = transcription_harness();
    let mut non_string = probe_request("azure", MODEL, JsonObject::new());
    non_string.model_params.insert(
        String::from("realtime_protocol"),
        Json::Number(serde_json::Number::from(1_i64)),
    );
    harness.run(&non_string, &beta_environment()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a truthy non-string model protocol suppresses the environment and falls back to GA"
    );
}

#[tokio::test]
async fn realtime_probe_azure_truthy_list_is_not_a_protocol_string() {
    let harness = transcription_harness();
    let truthy = probe_request(
        "azure",
        MODEL,
        params(&[("realtime_protocol", Json::List(vec![text("GA")]))]),
    );
    harness.run(&truthy, &beta_environment()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a non-empty list is truthy and is not a string"
    );
}

#[tokio::test]
async fn realtime_probe_azure_truthy_object_is_not_a_protocol_string() {
    let harness = transcription_harness();
    let truthy = probe_request(
        "azure",
        MODEL,
        params(&[(
            "realtime_protocol",
            Json::Object(JsonObject::from([(String::from("value"), text("GA"))])),
        )]),
    );
    harness.run(&truthy, &beta_environment()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/v1/realtime?intent=transcription",
        "a non-empty object is truthy and is not a string"
    );
}

#[tokio::test]
async fn realtime_probe_azure_falsy_protocol_values_fall_through() {
    let harness = transcription_harness();
    let mut empty_named = probe_request("azure", MODEL, JsonObject::new());
    empty_named.realtime_protocol = Some(String::new());
    harness
        .run(&empty_named, &beta_environment())
        .await
        .unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty named protocol is absent, so the environment applies"
    );
    let empty_list = probe_request(
        "azure",
        MODEL,
        params(&[("realtime_protocol", Json::List(Vec::new()))]),
    );
    harness.run(&empty_list, &beta_environment()).await.unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty list is falsy"
    );
}

#[tokio::test]
async fn realtime_probe_azure_empty_object_is_falsy() {
    let harness = transcription_harness();
    let empty_object = probe_request(
        "azure",
        MODEL,
        params(&[("realtime_protocol", Json::Object(JsonObject::new()))]),
    );
    harness
        .run(&empty_object, &beta_environment())
        .await
        .unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty object is falsy"
    );
}

#[tokio::test]
async fn realtime_probe_azure_empty_version_takes_the_default() {
    let harness = transcription_harness();
    let empty_version = probe_request("azure", MODEL, params(&[("api_version", text(""))]));
    harness
        .run(&empty_version, &beta_environment())
        .await
        .unwrap();
    assert_eq!(
        harness.dialer.last().url,
        "/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription",
        "an empty configured version takes the default"
    );
}
