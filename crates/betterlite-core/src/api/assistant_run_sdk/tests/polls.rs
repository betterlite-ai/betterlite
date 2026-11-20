#![cfg(test)]

use secrecy::ExposeSecret as _;

use super::*;

#[tokio::test]
async fn openai_poll_posts_the_thread_path_and_optionals() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(200, QUEUED);
    recorder.queue(200, COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.additional_instructions = Some(String::from("extra"));
    input.instructions = Some(String::from("base"));
    input.metadata = Some(serde_json::json!({"tag": "x"}));
    input.model = Some(String::from("gpt-4o-mini"));
    input.tools = Some(serde_json::json!([{"type": "code_interpreter"}]));
    input.stream = flag(false);
    let reply = polled(sdk.run_thread(input).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_a/runs"));
    assert_eq!(seen[1].method, "GET");
    assert_eq!(seen[1].url, format!("{BASE}/threads/thread_a/runs/run_1"));
    let body = seen[0].body.clone().unwrap();
    assert_eq!(body["assistant_id"], "asst_a");
    assert_eq!(body["additional_instructions"], "extra");
    assert_eq!(body["instructions"], "base");
    assert_eq!(body["metadata"]["tag"], "x");
    assert_eq!(body["model"], "gpt-4o-mini");
    assert_eq!(body["tools"][0]["type"], "code_interpreter");
    assert_eq!(body["stream"], false);
    assert!(body.get("thread_id").is_none());
    assert_eq!(seen[0].deadline, Duration::from_secs(600));
}

#[tokio::test]
async fn azure_poll_posts_its_thread_path_and_api_version() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(200, QUEUED);
    recorder.queue(200, COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut params = RunParams::default();
    params.api_base = Some(String::from(BASE));
    params.api_version = Some(String::from("2024-02-15-preview"));
    params.api_key = secret("az-key");
    let reply = polled(sdk.run_thread(azure_call(params)).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        format!("{BASE}/openai/threads/thread_a/runs?api-version=2024-02-15-preview")
    );
    assert_eq!(seen[0].api_key.as_deref(), Some("az-key"));
    assert!(seen[0].authorization.is_none());
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
    let body = seen[0].body.clone().unwrap();
    assert_eq!(body["assistant_id"], "asst_a");
    assert_eq!(body["stream"], false);
}

#[tokio::test]
async fn queued_then_completed_polls_again_with_the_same_deadline() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(200, QUEUED);
    recorder.queue(200, COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let reply = polled(sdk.run_thread(call("openai")).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].deadline, Duration::from_secs(600));
    assert_eq!(seen[1].deadline, Duration::from_secs(600));
}

#[tokio::test]
async fn only_the_literal_true_streams() {
    let values = [
        Some(None),
        flag(false),
        Some(Some(RunFlag::Number(1.0_f64))),
        Some(Some(RunFlag::Text(String::from("true")))),
    ];
    for stream in values {
        let (recorder, fixture) = openai_recorder();
        recorder.queue_twice(COMPLETED);
        let sdk = fixture.sdk(ProviderRuns {
            transport: Arc::clone(&recorder),
        });
        let mut input = call("openai");
        input.stream = stream;
        let reply = polled(sdk.run_thread(input).await.unwrap()).unwrap();
        assert_eq!(reply.status, "completed");
        assert_eq!(
            recorder.requests()[0].body.clone().unwrap()["stream"],
            false
        );
    }
}

#[tokio::test]
async fn arun_thread_flag_values_keep_sync_dispatch() {
    let values = [
        None,
        Some(RunFlag::Bool(false)),
        Some(RunFlag::Number(1.0_f64)),
        Some(RunFlag::Text(String::from("true"))),
    ];
    for value in values {
        let fixture = Fixture::new();
        let operations = CaptureRuns::default();
        let sdk = fixture.sdk(operations);
        let mut input = call("openai");
        input.arun_thread = value;
        assert!(polled_outcome(sdk.run_thread(input).await.unwrap()).is_none());
        match sdk.operations.targets().remove(0) {
            CapturedTarget::OpenAi { kind, .. } => assert_eq!(kind, ClientKind::Sync),
            CapturedTarget::Azure { .. } => panic!("the openai provider built an azure target"),
        }
    }
}

#[tokio::test]
async fn a_null_poll_result_passes_through() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    assert!(polled_outcome(sdk.run_thread(call("openai")).await.unwrap()).is_none());
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    assert!(polled_outcome(sdk.arun_thread(call("openai")).await.unwrap()).is_none());
}

#[tokio::test]
async fn openai_settings_chain_and_organization_reach_the_request() {
    let (recorder, mut fixture) = openai_recorder();
    fixture.settings.api_key = secret("sk-settings");
    fixture.settings.openai_organization = secret_text("org_settings");
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.params.api_key = secret("sk-request");
    input.params.api_base = Some(String::from(BASE));
    let reply = polled(sdk.run_thread(input).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-request"));
    assert_eq!(seen[0].organization.as_deref(), Some("org_settings"));
    assert_eq!(seen[0].url, format!("{BASE}/threads/thread_a/runs"));
}

#[tokio::test]
async fn an_empty_request_key_falls_back_to_the_settings_key() {
    let (recorder, mut fixture) = openai_recorder();
    fixture.settings.api_key = secret("sk-settings");
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.params.api_key = secret("");
    let reply = polled(sdk.run_thread(input).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    assert_eq!(
        recorder.requests()[0].authorization.as_deref(),
        Some("Bearer sk-settings")
    );
}

#[tokio::test]
async fn the_openai_base_defaults_to_the_public_endpoint() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let reply = polled(sdk.run_thread(call("openai")).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    assert_eq!(
        recorder.requests()[0].url,
        "https://api.openai.com/v1/threads/thread_a/runs"
    );
}

#[tokio::test]
async fn azure_ad_token_is_sent_as_a_bearer() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut params = RunParams::default();
    params.api_base = Some(String::from(BASE));
    params.azure_ad_token = secret("tok");
    let reply = polled(sdk.run_thread(azure_call(params)).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer tok"));
    assert!(seen[0].api_key.is_none());
}

#[tokio::test]
async fn an_extra_body_token_never_authenticates() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut params = RunParams::default();
    params.api_base = Some(String::from(BASE));
    params.api_key = secret("az-key");
    params.extra_body = Some(serde_json::json!({"azure_ad_token": "body-token"}));
    let reply = polled(sdk.run_thread(azure_call(params)).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(seen[0].api_key.as_deref(), Some("az-key"));
    assert!(seen[0].authorization.is_none());
}

#[tokio::test]
async fn azure_timeout_becomes_the_client_deadline() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut params = RunParams::default();
    params.api_base = Some(String::from(BASE));
    params.api_key = secret("az-key");
    params.timeout = Some(RunTimeout::Plain(TimeoutValue::Number(2.0_f64)));
    let reply = polled(sdk.run_thread(azure_call(params)).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    assert_eq!(recorder.requests()[0].deadline, Duration::from_secs(2));
}

#[tokio::test]
async fn azure_default_timeout_is_ten_minutes() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut params = RunParams::default();
    params.api_base = Some(String::from(BASE));
    params.api_key = secret("az-key");
    let reply = polled(sdk.run_thread(azure_call(params)).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    assert_eq!(recorder.requests()[0].deadline, Duration::from_secs(600));
}

#[tokio::test]
async fn max_retries_reach_both_providers() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut input = call("openai");
    input.params.max_retries = Some(2);
    sdk.run_thread(input).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::OpenAi { options, .. } => assert_eq!(options.max_retries, Some(2)),
        CapturedTarget::Azure { .. } => panic!("the openai provider built an azure target"),
    }
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut params = RunParams::default();
    params.max_retries = Some(3);
    sdk.run_thread(azure_call(params)).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::Azure {
            params,
            max_retries,
            ..
        } => {
            assert_eq!(params.max_retries, Some(3));
            assert_eq!(max_retries, Some(3));
        }
        CapturedTarget::OpenAi { .. } => panic!("the azure provider built an openai target"),
    }
}

#[tokio::test]
async fn azure_base_key_and_version_stay_optional_strings() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut params = RunParams::default();
    params.api_base = Some(String::from(BASE));
    params.api_key = secret("az-key");
    params.api_version = Some(String::from("2024-02-15-preview"));
    sdk.run_thread(azure_call(params)).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::Azure {
            api_key,
            api_base,
            api_version,
            ..
        } => {
            assert_eq!(api_base.as_deref(), Some(BASE));
            assert_eq!(api_version.as_deref(), Some("2024-02-15-preview"));
            assert_eq!(
                api_key.as_ref().map(|key| key.expose_secret().as_str()),
                Some("az-key")
            );
        }
        CapturedTarget::OpenAi { .. } => panic!("the azure provider built an openai target"),
    }
}

#[tokio::test]
async fn a_supplied_client_timeout_becomes_the_poll_deadline() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.client = Some(RunClient {
        class: RunClientClass::OpenAi,
        api_key: secret("client-key"),
        base_url: Some(String::from(BASE)),
        timeout: Some(3.0_f64),
        ..RunClient::default()
    });
    let reply = polled(sdk.run_thread(input).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    assert_eq!(recorder.requests()[0].deadline, Duration::from_secs(3));
}

#[tokio::test]
async fn a_supplied_azure_client_reaches_the_request() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(200, QUEUED);
    recorder.queue(200, COMPLETED);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let mut input = call("openai");
    input.client = Some(RunClient {
        class: RunClientClass::Azure,
        api_key: secret("client-key"),
        base_url: Some(String::from(BASE)),
        api_version: Some(String::from("2024-02-15-preview")),
        azure_ad_token: secret("client-token"),
        timeout: Some(5.0_f64),
        ..RunClient::default()
    });
    let reply = polled(sdk.run_thread(input).await.unwrap()).unwrap();
    assert_eq!(reply.status, "completed");
    let seen = recorder.requests();
    assert_eq!(
        seen[0].url,
        format!("{BASE}/threads/thread_a/runs?api-version=2024-02-15-preview")
    );
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer client-token")
    );
    assert!(seen[0].api_key.is_none());
    assert_eq!(seen[0].deadline, Duration::from_secs(5));
}

#[tokio::test]
async fn litellm_params_reach_only_the_azure_branch() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut params = RunParams::default();
    params.timeout = Some(RunTimeout::Plain(TimeoutValue::Number(2.0_f64)));
    params.max_retries = Some(1);
    params.azure_ad_token = secret("raw-token");
    sdk.run_thread(azure_call(params)).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::Azure { params, .. } => {
            assert_eq!(params.timeout, Some(2.0_f64));
            assert_eq!(params.max_retries, Some(1));
            assert_eq!(
                params
                    .azure_ad_token
                    .as_ref()
                    .map(|value| value.expose_secret().as_str()),
                Some("raw-token")
            );
        }
        CapturedTarget::OpenAi { .. } => panic!("the azure provider built an openai target"),
    }
}
