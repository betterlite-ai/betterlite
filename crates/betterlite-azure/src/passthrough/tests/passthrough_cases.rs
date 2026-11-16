use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_openai::api::responses_event_records::records::ResponsesTerminalEvent;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::RelayCallType;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::passthrough::config::{
    AzurePassthroughConfig, AzurePassthroughEnvironment, validate_environment,
};
use crate::passthrough::endpoint;
use crate::passthrough::logging::{
    AzureRelayedLog, NoChatReplyProjection, NoChatStreamAssembly, relayed_messages,
};
use crate::passthrough::url::without_api_version;

use super::nonstream_cases::nonstream_usage_case;
use super::support::{
    NoSources, RecordingLogging, RecordingStream, base_config, chat_chunks, complete, deployment,
    headers, object, secret, target, terminal_chunks,
};

const RESOURCE: &str = "https://my-resource.openai.azure.com";
pub(super) const CHAT_ENDPOINT: &str = "openai/deployments/gpt-4.1-mini/chat/completions";
const FULL_URL_BASE: &str = "https://my-resource.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions?api-version=2024-10-21";
pub(super) const EMBEDDINGS_ENDPOINT: &str = "openai/deployments/text-embedding-3-small/embeddings";
pub(super) const RESPONSES_ENDPOINT: &str = "openai/responses";
const NO_SOURCES: NoSources = NoSources;

#[rstest]
#[case::case_1_body_model_inference_endpoint(body_model_inference_endpoint_case)]
#[case::case_2_router_model_in_path(router_model_in_path_case)]
#[case::case_3_foreign_deployment(foreign_deployment_case)]
#[case::case_4_api_version_removal(api_version_removal_case)]
#[case::case_5_responses_stream_log(responses_stream_log_case)]
#[case::case_6_messages_relay_log(messages_relay_log_case)]
#[case::case_7_nonstream_usage(nonstream_usage_case)]
#[case::case_8_passthrough_auth_inputs(passthrough_auth_inputs_case)]
fn passthrough_cases(#[case] case: fn()) {
    case();
}

fn body_model_inference_endpoint_case() {
    body_model_inference_endpoints();
    streaming_flags();
}

fn body_model_inference_endpoints() {
    let table: [(&str, bool); 18] = [
        ("openai/v1/responses", true),
        ("openai/responses", true),
        ("/openai/v1/chat/completions/", true),
        ("openai/v1/embeddings", true),
        ("models/chat/completions", true),
        ("openai/v1/audio/speech", true),
        ("completions", true),
        ("openai/deployments/gpt-5.4/chat/completions", false),
        ("openai/deployments/gpt-5.4/responses", false),
        ("openai/v1/fine_tuning/jobs", false),
        ("openai/v1/assistants", false),
        ("openai/v1/responses/resp_123", false),
        ("openai/v1/batches", false),
        ("openai/deployments//chat/completions", true),
        ("xopenai/deployments/gpt/chat/completions", true),
        ("openai/deployments/gpt", false),
        ("", false),
        ("openai/images/generations", true),
    ];
    for (value, expected) in table {
        assert_eq!(
            endpoint::body_model_inference_endpoint(value),
            expected,
            "{value}"
        );
    }
}

fn streaming_flags() {
    let config = base_config(None);
    let table: [(&str, bool); 12] = [
        (r#"{"stream": true}"#, true),
        (r#"{"stream": 1}"#, true),
        (r#"{"stream": false}"#, false),
        ("{}", false),
        (r#"{"stream": 0}"#, false),
        (r#"{"stream": ""}"#, false),
        (r#"{"stream": "false"}"#, true),
        (r#"{"stream": null}"#, false),
        (r#"{"stream": []}"#, false),
        (r#"{"stream": [0]}"#, true),
        (r#"{"stream": {}}"#, false),
        (r#"{"stream": {"x": 1}}"#, true),
    ];
    for (request_data, expected) in table {
        assert_eq!(
            config.is_streaming_request(CHAT_ENDPOINT, &object(request_data)),
            expected,
            "{request_data}"
        );
    }
}

fn router_model_in_path_case() {
    let models = [String::from("gpt"), String::from("other-group")];
    let table: [(&str, Option<&str>); 9] = [
        (
            "other-group/openai/deployments/other-group/chat/completions",
            Some("other-group"),
        ),
        ("openai/deployments/gpt/chat/completions", Some("gpt")),
        (
            "openai/deployments/my-azure-deployment/chat/completions",
            None,
        ),
        ("gpt", None),
        ("", None),
        ("//", None),
        ("GPT/chat/completions", None),
        ("gpt/other-group/chat/completions", Some("gpt")),
        ("gpt/", Some("gpt")),
    ];
    for (value, expected) in table {
        assert_eq!(
            endpoint::router_model_in_endpoint(value, &models),
            expected,
            "{value}"
        );
    }
    assert_eq!(endpoint::router_model_in_endpoint("gpt/x", &[]), None);
}

fn foreign_deployment_case() {
    foreign_deployment_table();
    foreign_deployment_skips_the_router();
    foreign_deployment_casefolds_served_names();
    foreign_deployment_keeps_the_segment_as_sent();
    foreign_deployment_ignores_empty_segments();
}

fn foreign_deployment_table() {
    let served = [String::from("gpt-5.4-mini")];
    let table: [(&str, Option<&str>); 10] = [
        ("gpt/openai/deployments/gpt/chat/completions", None),
        ("openai/deployments/gpt/chat/completions", None),
        ("gpt/openai/deployments/gpt-5.4-mini/chat/completions", None),
        ("gpt/openai/deployments/GPT-5.4-MINI/chat/completions", None),
        ("gpt/openai/deployments/Gpt/chat/completions", Some("Gpt")),
        ("gpt/models/chat/completions", None),
        (
            "gpt/openai/deployments/gpt-5.4/chat/completions",
            Some("gpt-5.4"),
        ),
        (
            "gpt/openai/deployments/other-group/chat/completions",
            Some("other-group"),
        ),
        (
            "openai/deployments/victim/gpt/chat/completions",
            Some("victim"),
        ),
        (
            "gpt/openai/deployments/GPT-5.4/chat/completions",
            Some("GPT-5.4"),
        ),
    ];
    for (value, expected) in table {
        assert_eq!(
            endpoint::foreign_azure_deployment(value, "gpt", || served.to_vec()).as_deref(),
            expected,
            "{value}"
        );
    }
}

fn foreign_deployment_skips_the_router() {
    let skipped =
        || -> Vec<String> { panic!("the router must not be consulted for the group's own name") };
    assert_eq!(
        endpoint::foreign_azure_deployment(
            "gpt/openai/deployments/gpt/chat/completions",
            "gpt",
            skipped
        ),
        None
    );
    let no_deployment = || -> Vec<String> { panic!("there is no deployment name to compare") };
    assert_eq!(
        endpoint::foreign_azure_deployment("gpt/models/chat/completions", "gpt", no_deployment),
        None
    );
}

fn foreign_deployment_casefolds_served_names() {
    assert_eq!(
        endpoint::foreign_azure_deployment(
            "gpt/openai/deployments/STRASSE/chat/completions",
            "gpt",
            || vec![String::from("straße")]
        ),
        None
    );
    let no_served = || Vec::new();
    assert_eq!(
        endpoint::foreign_azure_deployment(
            "gpt/openai/deployments/GPT/chat/completions",
            "gpt",
            no_served
        ),
        Some(String::from("GPT"))
    );
}

fn foreign_deployment_keeps_the_segment_as_sent() {
    let no_served = || Vec::new();
    assert_eq!(
        endpoint::foreign_azure_deployment(
            "gpt/openai/deployments/gpt%2Fmini/chat/completions",
            "gpt",
            no_served
        ),
        Some(String::from("gpt%2Fmini"))
    );
    let again = || Vec::new();
    assert_eq!(
        endpoint::foreign_azure_deployment(
            "soloopenai/deployments/x/gpt/openai/deployments/victim/chat/completions",
            "gpt",
            again
        ),
        Some(String::from("victim"))
    );
}

fn foreign_deployment_ignores_empty_segments() {
    let no_deployment = || -> Vec<String> { panic!("an empty segment is not a deployment name") };
    assert_eq!(
        endpoint::foreign_azure_deployment(
            "openai/deployments//chat/completions",
            "gpt",
            no_deployment
        ),
        None
    );
}

fn api_version_removal_case() {
    api_version_stripping_table();
    caller_version_wins();
    deployment_version_fills_in();
    base_version_is_replaced_by_the_caller();
    base_version_survives_without_a_caller();
    leading_model_segment_is_stripped();
    model_group_rewrites_one_segment();
    numeric_versions_are_text();
    wrong_version_types_name_the_field();
    environment_base_fallbacks();
}

fn api_version_stripping_table() {
    let table: [(&str, &str); 5] = [
        (
            "https://x.test/openai/responses?api-version=2024-10-21",
            "https://x.test/openai/responses",
        ),
        (
            "https://x.test/p?a=1&api-version=2&b=3",
            "https://x.test/p?a=1&b=3",
        ),
        (
            "https://x.test/p?a=1&b=2&a=3&api-version=9",
            "https://x.test/p?a=1&a=3&b=2",
        ),
        ("https://x.test/p?api-version=1", "https://x.test/p"),
        (
            "https://x.test/p?q=a+b&r=%2F*z",
            "https://x.test/p?q=a+b&r=%2F%2Az",
        ),
    ];
    for (api_base, expected) in table {
        assert_eq!(
            without_api_version(api_base).unwrap().as_str(),
            expected,
            "{api_base}"
        );
    }
}

fn caller_version_wins() {
    let caller = object(r#"{"api-version": "2025-04-01-preview"}"#);
    let params = object(r#"{"api_version": "2024-10-21"}"#);
    let config = base_config(None);
    let (url, base_target) = complete(
        &config,
        Some(RESOURCE),
        target("gpt-4.1-mini", CHAT_ENDPOINT),
        Some(&caller),
        &params,
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        format!("{RESOURCE}/{CHAT_ENDPOINT}?api-version=2025-04-01-preview")
    );
    assert_eq!(base_target, RESOURCE);
}

fn deployment_version_fills_in() {
    let caller = object("{}");
    let params = object(r#"{"api_version": "2024-10-21"}"#);
    let config = base_config(None);
    let (url, _base) = complete(
        &config,
        Some(RESOURCE),
        target("gpt-4.1-mini", CHAT_ENDPOINT),
        Some(&caller),
        &params,
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        "https://my-resource.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions?api-version=2024-10-21"
    );
}

fn base_version_is_replaced_by_the_caller() {
    let caller = object(r#"{"api-version": "2025-04-01-preview"}"#);
    let config = base_config(None);
    let (url, base_target) = complete(
        &config,
        Some(FULL_URL_BASE),
        target("gpt-4.1-mini", "chat/completions"),
        Some(&caller),
        &object("{}"),
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        "https://my-resource.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions?api-version=2025-04-01-preview"
    );
    assert_eq!(base_target, FULL_URL_BASE);
}

fn base_version_survives_without_a_caller() {
    let config = base_config(None);
    let (url, _base) = complete(
        &config,
        Some(FULL_URL_BASE),
        target("gpt-4.1-mini", "chat/completions"),
        Some(&object("{}")),
        &object("{}"),
    )
    .unwrap();
    assert_eq!(url.as_str(), FULL_URL_BASE);
}

fn leading_model_segment_is_stripped() {
    let caller = object(r#"{"api-version": "2024-10-21"}"#);
    let config = base_config(None);
    let (url, _base) = complete(
        &config,
        Some(RESOURCE),
        target(
            "gpt-4.1-mini",
            "gpt-4.1-mini/openai/deployments/gpt-4.1-mini/chat/completions",
        ),
        Some(&caller),
        &object("{}"),
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        "https://my-resource.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions?api-version=2024-10-21"
    );
}

fn model_group_rewrites_one_segment() {
    let caller = object(r#"{"api-version": "2024-10-21"}"#);
    let params = object(r#"{"litellm_metadata": {"model_group": "gpt"}}"#);
    let config = base_config(None);
    let (url, _base) = complete(
        &config,
        Some(RESOURCE),
        target(
            "gpt-4.1-mini",
            "gpt/openai/deployments/gpt-4.1-mini/chat/completions",
        ),
        Some(&caller),
        &params,
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        "https://my-resource.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions?api-version=2024-10-21"
    );
}

fn numeric_versions_are_text() {
    let caller = object(r#"{"api-version": 20241021}"#);
    let config = base_config(None);
    let (url, _base) = complete(
        &config,
        Some(RESOURCE),
        target("gpt-4.1-mini", CHAT_ENDPOINT),
        Some(&caller),
        &object("{}"),
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        format!("{RESOURCE}/{CHAT_ENDPOINT}?api-version=20241021")
    );
    let bad_params = object(r#"{"api_version": ["2024"]}"#);
    let (url, _base) = complete(
        &config,
        Some(RESOURCE),
        target("gpt-4.1-mini", CHAT_ENDPOINT),
        Some(&caller),
        &bad_params,
    )
    .unwrap();
    assert_eq!(
        url.as_str(),
        format!("{RESOURCE}/{CHAT_ENDPOINT}?api-version=20241021")
    );
}

fn wrong_version_types_name_the_field() {
    let nested = object(r#"{"api-version": {"x": "2024"}}"#);
    let error = complete(
        &base_config(None),
        Some(RESOURCE),
        target("m", CHAT_ENDPOINT),
        Some(&nested),
        &object("{}"),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the query parameter api-version is not a string or a number"
    );
    let listed = object(r#"{"api_version": ["2024"]}"#);
    let error = complete(
        &base_config(None),
        Some(RESOURCE),
        target("m", CHAT_ENDPOINT),
        None,
        &listed,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the query parameter api-version is not a string or a number"
    );
}

fn environment_base_fallbacks() {
    environment_base_fallback();
    explicit_base_beats_the_environment();
    empty_environment_base_is_unset();
    missing_base_is_named();
}

fn environment_base_fallback() {
    let config = base_config(Some("https://env.test"));
    let (url, base_target) = complete(
        &config,
        None,
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap();
    assert_eq!(base_target, "https://env.test");
    assert_eq!(url.as_str(), "https://env.test/chat/completions");
}

fn explicit_base_beats_the_environment() {
    let config = base_config(Some("https://env.test"));
    let (_url, base_target) = complete(
        &config,
        Some("https://caller.test"),
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap();
    assert_eq!(base_target, "https://caller.test");
}

fn empty_environment_base_is_unset() {
    let config = base_config(Some(""));
    let error = complete(
        &config,
        None,
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Azure api base not found");
}

fn missing_base_is_named() {
    let error = complete(
        &base_config(None),
        None,
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Azure api base not found");
}

fn responses_stream_log_case() {
    terminal_event_is_logged();
    streams_without_a_terminal_event_log_nothing();
    responses_path_check_precedes_chat();
    unknown_stream_endpoints_log_nothing();
}

fn terminal_event_is_logged() {
    let chunks = terminal_chunks("resp_1");
    let mut logging = RecordingLogging::default();
    let logged = base_config(None).handle_collected_chunks(
        &chunks,
        &mut logging,
        &target("gpt-4.1-mini", RESPONSES_ENDPOINT),
    );
    let Some(AzureRelayedLog::Terminal(event)) = logged else {
        panic!("a terminal event must be the logged response");
    };
    let ResponsesTerminalEvent::Completed(completed) = *event else {
        panic!("the fixture carries a response.completed event");
    };
    assert_eq!(completed.response.id, "resp_1");
    assert_eq!(completed.response.created_at, 7);
    assert!(completed.extra.contains_key("sequence_number"));
    assert!(completed.response.extra.contains_key("x-extra"));
    assert_eq!(
        logging.call_type.map(RelayCallType::as_str),
        Some("aresponses")
    );
}

fn streams_without_a_terminal_event_log_nothing() {
    let chunks = vec![
        String::from(
            r#"data: {"type": "response.output_text.delta", "sequence_number": 1, "item_id": "msg_1", "delta": "hi"}"#,
        ),
        String::from("data: [DONE]"),
    ];
    let mut logging = RecordingLogging::default();
    let logged = base_config(None).handle_collected_chunks(
        &chunks,
        &mut logging,
        &target("gpt-4.1-mini", RESPONSES_ENDPOINT),
    );
    assert!(logged.is_none());
    assert_eq!(logging.call_type, None);
}

fn responses_path_check_precedes_chat() {
    let chunks = terminal_chunks("resp_2");
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: RecordingStream::default(),
    };
    let mut logging = RecordingLogging::default();
    let endpoint_path = "openai/deployments/x/chat/completions/responses";
    let logged = config.handle_collected_chunks(
        &chunks,
        &mut logging,
        &target("gpt-4.1-mini", endpoint_path),
    );
    assert!(matches!(logged, Some(AzureRelayedLog::Terminal(_))));
    assert!(config.chat_stream.taken().is_none());
    assert_eq!(
        logging.call_type.map(RelayCallType::as_str),
        Some("aresponses")
    );
}

fn unknown_stream_endpoints_log_nothing() {
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: RecordingStream::default(),
    };
    let mut logging = RecordingLogging::default();
    let logged = config.handle_collected_chunks(
        &chat_chunks(),
        &mut logging,
        &target("text-embedding-3-small", EMBEDDINGS_ENDPOINT),
    );
    assert!(logged.is_none());
    assert!(config.chat_stream.taken().is_none());
    assert_eq!(logging.call_type, None);
}

fn messages_relay_log_case() {
    relayed_messages_validation_table();
    chat_chunks_reach_the_assembler();
    chat_chunks_reach_it_without_valid_messages();
    non_chat_streams_never_reach_the_assembler();
}

fn relayed_messages_validation_table() {
    let kept = relayed_messages(&object(
        r#"{"request_data": {"messages": [{"role": "user"}], "stream": true}}"#,
    ));
    assert_eq!(kept, Some(vec![object(r#"{"role": "user"}"#)]));
    assert_eq!(
        relayed_messages(&object(r#"{"request_data": {"messages": []}}"#)),
        Some(Vec::new())
    );
    assert_eq!(
        relayed_messages(&object(r#"{"request_data": {"messages": "hi"}}"#)),
        None
    );
    assert_eq!(
        relayed_messages(&object(r#"{"request_data": {"messages": [1]}}"#)),
        None
    );
    assert_eq!(
        relayed_messages(&object(r#"{"request_data": {"messages": null}}"#)),
        None
    );
    assert_eq!(
        relayed_messages(&object(r#"{"request_data": {"messages": {}}}"#)),
        None
    );
    assert_eq!(relayed_messages(&object(r#"{"request_data": "hi"}"#)), None);
    assert_eq!(relayed_messages(&object(r#"{"request_data": null}"#)), None);
    assert_eq!(relayed_messages(&object("{}")), None);
}

fn chat_chunks_reach_the_assembler() {
    let details = object(
        r#"{"request_data": {"messages": [{"role": "user", "content": "hi"}], "stream": true}}"#,
    );
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: Some(details),
        chat_reply: NoChatReplyProjection,
        chat_stream: RecordingStream::default(),
    };
    let chunks = chat_chunks();
    let mut logging = RecordingLogging::default();
    let logged = config.handle_collected_chunks(
        &chunks,
        &mut logging,
        &target("gpt-4.1-mini", CHAT_ENDPOINT),
    );
    assert!(logged.is_none());
    let recorded = config.chat_stream.taken().unwrap();
    assert_eq!(recorded.model, "gpt-4.1-mini");
    assert_eq!(recorded.chunks, chunks);
    assert_eq!(
        recorded.messages,
        Some(vec![object(r#"{"role": "user", "content": "hi"}"#)])
    );
    assert_eq!(logging.call_type, None);
}

fn chat_chunks_reach_it_without_valid_messages() {
    let details = object(r#"{"request_data": {"messages": "hi"}}"#);
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: Some(details),
        chat_reply: NoChatReplyProjection,
        chat_stream: RecordingStream::default(),
    };
    let mut logging = RecordingLogging::default();
    let logged = config.handle_collected_chunks(
        &chat_chunks(),
        &mut logging,
        &target("gpt-4.1-mini", CHAT_ENDPOINT),
    );
    assert!(logged.is_none());
    assert_eq!(config.chat_stream.taken().unwrap().messages, None);
    let no_details = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: RecordingStream::default(),
    };
    let mut logging = RecordingLogging::default();
    no_details.handle_collected_chunks(
        &chat_chunks(),
        &mut logging,
        &target("gpt-4.1-mini", CHAT_ENDPOINT),
    );
    assert_eq!(no_details.chat_stream.taken().unwrap().messages, None);
}

fn non_chat_streams_never_reach_the_assembler() {
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: RecordingStream::default(),
    };
    let mut logging = RecordingLogging::default();
    let logged = config.handle_collected_chunks(
        &chat_chunks(),
        &mut logging,
        &target("text-embedding-3-small", EMBEDDINGS_ENDPOINT),
    );
    assert!(logged.is_none());
    assert!(config.chat_stream.taken().is_none());
}

fn passthrough_auth_inputs_case() {
    configured_keys_stay_per_instance();
    validation_headers_from_each_source();
    validation_header_precedence();
}

fn exposed(key: Option<&SecretString>) -> Option<String> {
    key.map(|key| key.expose_secret().to_owned())
}

fn environment<'data>(
    sent: &[(&str, &str)],
    api_key: Option<&'data SecretString>,
    configured_key: Option<&'data SecretString>,
    params_key: Option<&'data SecretString>,
    credentials: &'data DeploymentSettings,
) -> AzurePassthroughEnvironment<'data> {
    AzurePassthroughEnvironment {
        headers: headers(sent),
        api_key,
        params_key,
        configured_key,
        credentials,
        refresh_enabled: false,
        provider: None,
        sources: &NO_SOURCES,
    }
}

fn configured_keys_stay_per_instance() {
    let left = AzurePassthroughConfig {
        api_base_env: Some(String::from("https://left.test")),
        api_key_env: Some(secret("left-key")),
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    };
    let right = AzurePassthroughConfig {
        api_base_env: Some(String::from("https://right.test")),
        api_key_env: Some(secret("right-key")),
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    };
    assert_eq!(left.get_api_base(None), Some("https://left.test"));
    assert_eq!(right.get_api_base(None), Some("https://right.test"));
    assert_eq!(
        exposed(left.get_api_key(None)),
        Some(String::from("left-key"))
    );
    assert_eq!(
        exposed(right.get_api_key(None)),
        Some(String::from("right-key"))
    );
    let positional = secret("caller-key");
    assert_eq!(
        exposed(left.get_api_key(Some(&positional))),
        Some(String::from("caller-key"))
    );
    let empty = secret("");
    assert_eq!(
        exposed(left.get_api_key(Some(&empty))),
        Some(String::from("left-key"))
    );
}

fn validation_headers_from_each_source() {
    let positional = secret("positional-key");
    let credentials = deployment();
    let validated = validate_environment(environment(
        &[("x-probe", "1")],
        Some(&positional),
        None,
        None,
        &credentials,
    ))
    .unwrap();
    assert_eq!(
        validated.get("api-key").map(String::as_str),
        Some("positional-key")
    );
    assert_eq!(validated.get("x-probe").map(String::as_str), Some("1"));
    let sent = secret("sent-key");
    let kept = validate_environment(environment(
        &[("api-key", "sent-key")],
        Some(&sent),
        None,
        None,
        &credentials,
    ))
    .unwrap();
    assert_eq!(kept.get("api-key").map(String::as_str), Some("sent-key"));
}

fn validation_header_precedence() {
    let positional = secret("positional-key");
    let configured = secret("configured-key");
    let params = secret("params-key");
    let credentials = deployment();
    let from_positional = validate_environment(environment(
        &[],
        Some(&positional),
        Some(&configured),
        Some(&params),
        &credentials,
    ))
    .unwrap();
    assert_eq!(
        from_positional.get("api-key").map(String::as_str),
        Some("positional-key")
    );
    let from_globals_after_params = validate_environment(environment(
        &[],
        None,
        Some(&configured),
        Some(&params),
        &credentials,
    ))
    .unwrap();
    assert_eq!(
        from_globals_after_params.get("api-key").map(String::as_str),
        Some("configured-key")
    );
    let empty = secret("");
    let from_configured = validate_environment(environment(
        &[],
        Some(&empty),
        Some(&configured),
        Some(&params),
        &credentials,
    ))
    .unwrap();
    assert_eq!(
        from_configured.get("api-key").map(String::as_str),
        Some("configured-key")
    );
}
