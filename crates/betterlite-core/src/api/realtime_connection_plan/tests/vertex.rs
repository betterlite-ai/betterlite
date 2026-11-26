#![cfg(test)]

use secrecy::ExposeSecret as _;

use super::*;

const CHIRP_CATALOG: &[u8] = br#"{"vertex_ai/chirp_2": {"litellm_provider": "vertex_ai", "mode": "audio_transcription", "supported_endpoints": ["/v1/audio/transcriptions", "/v1/realtime"]}}"#;

// a Speech-to-Text model is rejected before any credential work
#[tokio::test]
async fn realtime_probe_vertex_and_failures() {
    let harness = Harness {
        vertex: FakeVertex::new(CHIRP_CATALOG),
        ..Harness::new()
    };
    let chirp = probe_request("vertex_ai", "chirp_2", JsonObject::new());
    let error = harness
        .run(&chirp, &settings())
        .await
        .expect_err("a Speech-to-Text model is rejected");
    assert_eq!(
        error.to_string(),
        "Realtime health checks are not supported for Speech-to-Text streaming model chirp_2; health check it with mode audio_transcription"
    );
    assert_eq!(
        harness.dialer.count(),
        0,
        "the rejection happens before any connect"
    );
}

#[tokio::test]
async fn realtime_probe_vertex_resolves_the_live_target() {
    let harness = Harness::new();
    let mut live = probe_request("vertex_ai", MODEL, JsonObject::new());
    live.api_key = Some(SecretString::from("caller-key"));
    live.api_base = Some(String::from("https://vertex-base.test"));
    harness.run(&live, &settings()).await.unwrap();
    let targets = harness.vertex.targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].model, MODEL);
    assert_eq!(targets[0].project, "resolved-project");
    assert_eq!(targets[0].region, "us-central1");
    assert_eq!(targets[0].access_token.expose_secret(), "oauth-token");
    assert_eq!(harness.vertex.configured_regions(), vec![None]);
    assert_eq!(
        harness.dialer.last().headers,
        Headers::from([(
            String::from("Authorization"),
            String::from("Bearer oauth-token")
        )]),
        "a caller API key never replaces the OAuth Bearer header"
    );
}

#[tokio::test]
async fn realtime_probe_vertex_timeout_and_unsupported_provider() {
    let harness = Harness::new();
    let live = probe_request("vertex_ai", MODEL, JsonObject::new());
    let mut timed_out = settings();
    timed_out.credential_timeout_seconds = 0.0_f64;
    let error = harness
        .run(&live, &timed_out)
        .await
        .expect_err("a zero bound expires without polling");
    assert_eq!(
        error.to_string(),
        "Vertex AI realtime: timed out fetching Google OAuth access token after 0.0s; check network egress from the proxy to the OAuth token endpoint (oauth2.googleapis.com)"
    );
    let unsupported = probe_request("bedrock", MODEL, JsonObject::new());
    let error = harness
        .run(&unsupported, &settings())
        .await
        .expect_err("an unsupported provider is a ValueError");
    assert_eq!(error.to_string(), "Unsupported model: realtime-a");
    assert_eq!(
        harness.dialer.count(),
        0,
        "neither failure reaches the dialer"
    );
}

#[tokio::test]
async fn realtime_probe_vertex_handshake_failures_propagate() {
    let harness = Harness::new();
    let live = probe_request("vertex_ai", MODEL, JsonObject::new());
    harness.run(&live, &settings()).await.unwrap();
    harness.dialer.fail(ProbeDialError::new(
        "InvalidStatus",
        String::from("server rejected WebSocket connection: HTTP 401"),
    ));
    let refused = harness
        .run(&live, &settings())
        .await
        .expect_err("a refused handshake propagates instead of returning false");
    match &refused {
        RealtimeProbeError::Dial(error) => assert_eq!(error.status, None),
        RealtimeProbeError::UnsupportedModel { .. }
        | RealtimeProbeError::VertexSpeechModel { .. }
        | RealtimeProbeError::ParameterType { .. }
        | RealtimeProbeError::Azure(_)
        | RealtimeProbeError::OpenAi(_)
        | RealtimeProbeError::VertexTokenTimeout { .. }
        | RealtimeProbeError::Token { .. } => {
            panic!("a refused handshake is a dial error, got {refused:?}")
        }
    }
    harness.dialer.fail(ProbeDialError::new(
        "InvalidStatus",
        String::from("HTTP 503"),
    ));
    harness
        .run(&live, &settings())
        .await
        .expect_err("every handshake failure propagates");
    assert_eq!(
        harness.dialer.count(),
        3,
        "the probe never retries a handshake: one live connect and two refused ones"
    );
}

#[tokio::test]
async fn realtime_probe_vertex_build_errors_propagate() {
    let harness = Harness::new();
    let mut broken = probe_request("openai", MODEL, JsonObject::new());
    broken.api_base = Some(String::from("::"));
    let error = harness
        .run(&broken, &settings())
        .await
        .expect_err("a URL build failure propagates");
    assert!(matches!(error, RealtimeProbeError::OpenAi(_)));
    assert_eq!(
        harness.dialer.count(),
        0,
        "a failed build never reaches the dialer"
    );
}
