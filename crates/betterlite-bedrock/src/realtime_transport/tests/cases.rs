use std::io::Error as IoError;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_session_state::logging::SUCCESS_LOGGED_KEY;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::capture::capture;
use super::logging::FakeQueue;
use super::support::{
    ACCESS, CONTENT_END, CONTENT_START, Chunk, FakeCredentials, FakeError, FakeRuntime, Fixture,
    MODEL, Options, REGION, RESOLVED_ACCESS, RESOLVED_SECRET, RESOLVED_TOKEN, ReceiveStep,
    RecordedConfig, RecordedResolve, SECRET, SESSION_UPDATE, ScriptedClient, StreamPlan,
    TEXT_OUTPUT, TOKEN, TRACE_ID, USAGE_EVENT, event_key, event_type, field, full_auth,
    full_session_tags, stream_error,
};
use crate::aws_settings::{AuthError, AuthParams};
use crate::credentials::error::CredentialError;
use crate::realtime_events::failure::ProviderStreamError;
use crate::realtime_transport::error::{NO_CREDENTIALS_MESSAGE, RealtimeTransportError};
use crate::realtime_transport::session::initial_state;
use crate::signing::target::RegionInputs;

pub(super) async fn bidirectional_runtime_target() {
    committed_scope_rejects_before_preparation().await;
    region_selection().await;
    endpoint_precedence().await;
    default_endpoint_is_regional().await;
    model_id_is_passed_through().await;
    connecting_log_names_the_endpoint_and_model().await;
}

async fn connecting_log_names_the_endpoint_and_model() {
    let (captured, _guard) = capture();
    let fixture = Fixture::new();
    let result = fixture.run(region_options(REGION)).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    let logs = captured.text();
    assert!(
        logs.contains(
            "Bedrock Realtime: Connecting to https://bedrock-runtime.us-east-1.amazonaws.com with model amazon.nova-sonic-v1:0"
        ),
        "{logs}"
    );
    assert!(
        logs.contains("Bedrock Realtime: Bidirectional stream established"),
        "{logs}"
    );
}

async fn region_selection() {
    explicit_region_is_retained().await;
    an_empty_explicit_region_is_retained().await;
    region_falls_back_through_the_environment().await;
    an_invalid_environment_region_is_rejected().await;
    region_falls_back_through_the_model_arn().await;
}

async fn config_for(fixture: &Fixture, options: Options<'_>) -> RecordedConfig {
    let result = fixture.run(options).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    fixture.last_config()
}

async fn committed_scope_rejects_before_preparation() {
    let fixture = Fixture::new();
    let result = fixture
        .run(Options {
            model: MODEL,
            pending: None,
            committed: true,
            committed_failure: Some("ModelStreamErrorException: the committed stream broke"),
            api_base: None,
            endpoint: None,
            region: Some(REGION),
            region_inputs: RegionInputs::default(),
            auth: AuthParams::default(),
            logged_types: LoggedEventTypes::Default,
        })
        .await;
    let error = result.outcome.unwrap_err();
    assert!(matches!(error, RealtimeTransportError::Committed(_)));
    assert_eq!(error.status_code(), Some(400));
    let message = error.to_string();
    assert!(message.contains("already committed"), "{message}");
    assert!(
        message.contains("ModelStreamErrorException: the committed stream broke"),
        "{message}"
    );
    assert!(
        fixture.runtime.configs().is_empty(),
        "the replay check runs before any client is built"
    );
    assert!(
        fixture.credentials.calls().is_empty(),
        "the replay check runs before any credential lookup"
    );
    assert!(fixture.client.sends().is_empty());
}

async fn explicit_region_is_retained() {
    let fixture = Fixture::new();
    let explicit = config_for(&fixture, region_options("eu-west-1")).await;
    assert_eq!(explicit.region, "eu-west-1");
    assert_eq!(
        explicit.endpoint_uri,
        "https://bedrock-runtime.eu-west-1.amazonaws.com"
    );

    let invalid = Fixture::new();
    let retained = config_for(&invalid, region_options("BAD_REGION")).await;
    assert_eq!(
        retained.region, "BAD_REGION",
        "an explicit region is never validated"
    );
}

async fn an_empty_explicit_region_is_retained() {
    let fixture = Fixture::new();
    let blank = config_for(&fixture, region_options("")).await;
    assert_eq!(blank.region, "", "an empty explicit region is retained");
    assert_eq!(
        blank.endpoint_uri, "https://bedrock-runtime..amazonaws.com",
        "the commercial partition answers an empty region"
    );
}

fn region_options(region: &str) -> Options<'_> {
    Options {
        model: MODEL,
        pending: None,
        committed: false,
        committed_failure: None,
        api_base: None,
        endpoint: None,
        region: Some(region),
        region_inputs: RegionInputs::default(),
        auth: AuthParams::default(),
        logged_types: LoggedEventTypes::Default,
    }
}

fn region_inputs(
    environment_region_name: Option<&'static str>,
    environment_region: Option<&'static str>,
    session_region: Option<&'static str>,
) -> RegionInputs<'static> {
    RegionInputs {
        environment_region_name,
        environment_region,
        session_region,
    }
}

fn environment_cases() -> [(RegionInputs<'static>, &'static str); 5] {
    [
        (
            region_inputs(Some("us-east-2"), Some("us-west-1"), Some("eu-north-1")),
            "us-east-2",
        ),
        (
            region_inputs(None, Some("us-west-1"), Some("eu-north-1")),
            "us-west-1",
        ),
        (region_inputs(None, None, Some("eu-north-1")), "eu-north-1"),
        (region_inputs(None, None, Some("")), "us-west-2"),
        (region_inputs(None, None, None), "us-west-2"),
    ]
}

async fn region_falls_back_through_the_environment() {
    for (inputs, expected) in environment_cases() {
        let fixture = Fixture::new();
        let config = config_for(
            &fixture,
            Options {
                model: MODEL,
                pending: None,
                committed: false,
                committed_failure: None,
                api_base: None,
                endpoint: None,
                region: None,
                region_inputs: inputs,
                auth: AuthParams::default(),
                logged_types: LoggedEventTypes::Default,
            },
        )
        .await;
        assert_eq!(config.region, expected);
        assert_eq!(fixture.credentials.calls()[0].region, expected);
    }
}

async fn an_invalid_environment_region_is_rejected() {
    let fixture = Fixture::new();
    let result = fixture
        .run(Options {
            model: MODEL,
            pending: None,
            committed: false,
            committed_failure: None,
            api_base: None,
            endpoint: None,
            region: None,
            region_inputs: region_inputs(Some("BAD_REGION"), None, None),
            auth: AuthParams::default(),
            logged_types: LoggedEventTypes::Default,
        })
        .await;
    let error = result.outcome.unwrap_err();
    assert!(matches!(error, RealtimeTransportError::Endpoint(_)));
    assert_eq!(error.status_code(), None);
    assert!(fixture.runtime.configs().is_empty());
}

async fn region_falls_back_through_the_model_arn() {
    let model = "arn:aws:bedrock:eu-central-1:123456789012:async-invoke/fixture";
    let fixture = Fixture::new();
    let config = config_for(
        &fixture,
        Options {
            model,
            pending: None,
            committed: false,
            committed_failure: None,
            api_base: None,
            endpoint: None,
            region: None,
            region_inputs: RegionInputs::default(),
            auth: AuthParams::default(),
            logged_types: LoggedEventTypes::Default,
        },
    )
    .await;
    assert_eq!(config.region, "eu-central-1");
    assert_eq!(
        fixture.runtime.invocations(),
        vec![model.to_owned()],
        "the ARN model id reaches the SDK without route stripping"
    );
}

async fn endpoint_precedence() {
    let cases: [(Option<&str>, Option<&str>, &str); 3] = [
        (
            Some("https://api-base.fixture"),
            Some("https://configured.fixture"),
            "https://api-base.fixture",
        ),
        (Some(""), Some("https://configured.fixture"), ""),
        (
            None,
            Some("https://configured.fixture"),
            "https://configured.fixture",
        ),
    ];
    for (api_base, endpoint, expected) in cases {
        let fixture = Fixture::new();
        let config = config_for(
            &fixture,
            Options {
                model: MODEL,
                pending: None,
                committed: false,
                committed_failure: None,
                api_base,
                endpoint,
                region: Some(REGION),
                region_inputs: RegionInputs::default(),
                auth: AuthParams::default(),
                logged_types: LoggedEventTypes::Default,
            },
        )
        .await;
        assert_eq!(config.endpoint_uri, expected);
    }
}

async fn default_endpoint_is_regional() {
    let fixture = Fixture::new();
    let config = config_for(&fixture, region_options(REGION)).await;
    assert_eq!(
        config.endpoint_uri,
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
}

async fn model_id_is_passed_through() {
    for model in [
        "us.amazon.nova-sonic-v1:0",
        "bedrock/amazon.nova-sonic-v1:0",
    ] {
        let fixture = Fixture::new();
        let _config = config_for(
            &fixture,
            Options {
                model,
                pending: None,
                committed: false,
                committed_failure: None,
                api_base: None,
                endpoint: None,
                region: Some(REGION),
                region_inputs: RegionInputs::default(),
                auth: AuthParams::default(),
                logged_types: LoggedEventTypes::Default,
            },
        )
        .await;
        assert_eq!(fixture.runtime.invocations(), vec![model.to_owned()]);
    }
}

pub(super) async fn initial_session_configuration() {
    created_event_when_nothing_is_pending().await;
    pending_update_suppresses_the_created_event().await;
    empty_pending_still_suppresses_the_created_event().await;
    fresh_transformation_state();
    initial_failures_escape_and_close().await;
}

async fn created_event_when_nothing_is_pending() {
    let fixture = Fixture::new();
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    let events = fixture.client.sent_events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(event_type(&events[0]), Some("session.created"));
    let session = field(&events[0], "session").expect("the session object");
    assert_eq!(field(session, "model"), Some(&Json::Text(MODEL.to_owned())));
    assert_eq!(field(session, "id"), Some(&Json::Text(TRACE_ID.to_owned())));
    assert_eq!(
        field(session, "modalities"),
        Some(&Json::List(vec![
            Json::Text("text".to_owned()),
            Json::Text("audio".to_owned()),
        ]))
    );
    assert!(matches!(
        field(&events[0], "event_id"),
        Some(Json::Text(event_id)) if !event_id.is_empty()
    ));
    assert!(
        fixture.runtime.stream().sent().is_empty(),
        "no Bedrock sessionStart or promptStart is sent before the client update"
    );
}

async fn pending_update_suppresses_the_created_event() {
    let fixture = Fixture::new();
    let result = fixture
        .run(Options {
            model: MODEL,
            pending: Some(SESSION_UPDATE),
            committed: false,
            committed_failure: None,
            api_base: None,
            endpoint: None,
            region: Some(REGION),
            region_inputs: RegionInputs::default(),
            auth: AuthParams::default(),
            logged_types: LoggedEventTypes::Default,
        })
        .await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    let events = fixture.client.sent_events();
    let created = events
        .iter()
        .filter(|event| event_type(event) == Some("session.created"));
    assert_eq!(created.count(), 0, "{events:?}");
    assert_eq!(event_type(&events[0]), Some("session.updated"));
    assert_eq!(result.scope.session_committed, Json::Bool(true));
    let sent = fixture.runtime.stream().sent();
    let keys: Vec<Option<&str>> = sent.iter().map(event_key).collect();
    assert_eq!(
        keys,
        vec![
            Some("sessionStart"),
            Some("promptStart"),
            Some("promptEnd"),
            Some("sessionEnd"),
        ],
        "the pending update is replayed, then the pump half-closes the input"
    );
    assert!(fixture.runtime.stream().closed());
}

async fn empty_pending_still_suppresses_the_created_event() {
    let fixture = Fixture::new();
    let result = fixture
        .run(Options {
            model: MODEL,
            pending: Some(""),
            committed: false,
            committed_failure: None,
            api_base: None,
            endpoint: None,
            region: Some(REGION),
            region_inputs: RegionInputs::default(),
            auth: AuthParams::default(),
            logged_types: LoggedEventTypes::Default,
        })
        .await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert!(
        fixture.client.sends().is_empty(),
        "an empty pending string still suppresses session.created"
    );
    assert!(fixture.runtime.stream().sent().is_empty());
    assert_eq!(result.scope.session_committed, Json::Null);
}

fn fresh_transformation_state() {
    let state = initial_state();
    assert!(state.session_configuration_request.is_none());
    assert!(state.current_output_item_id.is_none());
    assert!(state.current_response_id.is_none());
    assert!(state.current_delta_chunks.is_none());
    assert!(state.current_item_chunks.is_none());
    assert!(state.current_conversation_id.is_none());
    assert!(state.current_delta_type.is_none());
}

async fn initial_failures_escape_and_close() {
    let mut fixture = Fixture::new();
    fixture.runtime = FakeRuntime::default().failing_invoke(stream_error(
        "the invoke failed",
        FakeError::new("AccessDeniedException", "the fixture denied the invoke"),
    ));
    let result = fixture.run(Options::default()).await;
    let error = result.outcome.unwrap_err();
    assert!(
        matches!(&error, RealtimeTransportError::Stream(inner) if inner.message == "the invoke failed")
    );
    assert_eq!(error.status_code(), None);
    assert!(
        fixture.client.sends().is_empty(),
        "session.created never precedes a successful open"
    );
    assert_eq!(fixture.runtime.closes(), 1);
    assert_eq!(fixture.runtime.invocations(), vec![MODEL.to_owned()]);

    let mut created = Fixture::new();
    created.client = ScriptedClient::scripted(Vec::new()).failing_send(0);
    let send_result = created.run(Options::default()).await;
    assert!(matches!(
        send_result.outcome.unwrap_err(),
        RealtimeTransportError::Client(_)
    ));
    assert_eq!(created.client.sends().len(), 1);
    assert_eq!(created.runtime.closes(), 1);
    assert!(created.runtime.stream().sent().is_empty());
}

pub(super) async fn credential_failure() {
    auth_params_reach_the_resolver().await;
    resolved_credentials_reach_the_sdk().await;
    absent_credentials_answer_401().await;
    source_errors_propagate().await;
}

fn auth_options(auth: AuthParams) -> Options<'static> {
    Options {
        model: MODEL,
        pending: None,
        committed: false,
        committed_failure: None,
        api_base: None,
        endpoint: None,
        region: Some(REGION),
        region_inputs: RegionInputs::default(),
        auth,
        logged_types: LoggedEventTypes::Default,
    }
}

async fn auth_params_reach_the_resolver() {
    let fixture = Fixture::new();
    let result = fixture.run(auth_options(full_auth())).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert_resolved_auth(&fixture);
}

fn assert_resolved_auth(fixture: &Fixture) {
    let calls = fixture.credentials.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.region, REGION);
    assert_resolved_region_and_identity(call);
    assert_resolved_role(call);
    assert_absent_role_metadata(call);
}

fn assert_resolved_region_and_identity(call: &RecordedResolve) {
    assert_eq!(call.region_name.as_deref(), Some(REGION));
    assert_eq!(call.access_key_id.as_deref(), Some(ACCESS));
    assert_eq!(call.secret_access_key.as_deref(), Some(SECRET));
    assert_eq!(call.session_token.as_deref(), Some(TOKEN));
}

fn assert_resolved_role(call: &RecordedResolve) {
    assert_eq!(call.session_name.as_deref(), Some("fixture-session"));
    assert_eq!(call.profile_name.as_deref(), Some("fixture-profile"));
    assert_eq!(call.role_name.as_deref(), Some("fixture-role"));
    assert_eq!(
        call.web_identity_token.as_deref(),
        Some("fixture-web-identity")
    );
    assert_eq!(
        call.sts_endpoint.as_deref(),
        Some("https://sts.fixture.example")
    );
    assert_eq!(call.external_id.as_deref(), Some("fixture-external"));
    assert_eq!(call.session_tags.as_ref(), Some(&full_session_tags()));
}

fn assert_absent_role_metadata(call: &RecordedResolve) {
    assert!(call.role_arn.is_none());
    assert!(call.web_identity_token_file.is_none());
    assert!(call.bearer_token.is_none());
}

async fn resolved_credentials_reach_the_sdk() {
    let fixture = Fixture::new();
    let result = fixture.run(auth_options(full_auth())).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    let config = fixture.last_config();
    assert_eq!(
        config.access_key_id, RESOLVED_ACCESS,
        "the resolved credentials, not the request params, reach the SDK"
    );
    assert_eq!(config.secret_access_key, RESOLVED_SECRET);
    assert_eq!(config.session_token.as_deref(), Some(RESOLVED_TOKEN));
}

async fn absent_credentials_answer_401() {
    let mut fixture = Fixture::new();
    fixture.credentials = FakeCredentials::absent();
    let result = fixture.run(Options::default()).await;
    let error = result.outcome.unwrap_err();
    assert!(matches!(
        error,
        RealtimeTransportError::NoCredentials {
            status_code: 401,
            ..
        }
    ));
    assert_eq!(error.status_code(), Some(401));
    assert_eq!(error.to_string(), NO_CREDENTIALS_MESSAGE);
    assert!(fixture.runtime.configs().is_empty());
    assert_eq!(fixture.runtime.closes(), 0);
    assert!(fixture.client.sends().is_empty());
}

async fn source_errors_propagate() {
    let mut fixture = Fixture::new();
    fixture.credentials = FakeCredentials::failing(CredentialError::Auth(AuthError::SessionTags));
    let result = fixture.run(auth_options(full_auth())).await;
    let error = result.outcome.unwrap_err();
    assert!(matches!(error, RealtimeTransportError::Credential(_)));
    assert_eq!(error.status_code(), None);
    assert!(error.to_string().contains("aws_session_tags"));
    assert!(fixture.runtime.configs().is_empty());
}

pub(super) async fn upstream_session_failure() {
    leftover_usage_is_logged().await;
    rejected_enqueue_leaves_the_flag_unset().await;
    no_logged_events_leave_no_flag().await;
    provider_failure_is_raised_after_logging().await;
    a_failure_after_the_client_left_is_logged().await;
}

fn usage_plan() -> StreamPlan {
    StreamPlan::new(vec![Chunk::Text(USAGE_EVENT), Chunk::End])
}

async fn leftover_usage_is_logged() {
    let mut fixture = Fixture::new();
    fixture.runtime = FakeRuntime::default().with_plan(usage_plan());
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    let dispatched = fixture.logging.dispatched();
    assert_eq!(dispatched.len(), 1, "one success-handler dispatch");
    assert_eq!(dispatched[0].len(), 1);
    let record = &dispatched[0][0];
    assert_eq!(
        record.get("type"),
        Some(&Json::Text("response.done".to_owned()))
    );
    let total = record_field(record, &["response", "usage", "total_tokens"]).and_then(|value| {
        if let Json::Number(number) = value {
            number.as_i64()
        } else {
            None
        }
    });
    assert_eq!(total, Some(14), "the leftover usage delta is logged");
    assert_eq!(
        fixture.logging.detail(SUCCESS_LOGGED_KEY),
        Some(Json::Bool(true))
    );
    assert_eq!(fixture.queue.enqueued(), 1);
}

async fn rejected_enqueue_leaves_the_flag_unset() {
    let mut fixture = Fixture::new();
    fixture.runtime = FakeRuntime::default().with_plan(usage_plan());
    fixture.queue = FakeQueue::new(false);
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert_eq!(fixture.logging.dispatched().len(), 1);
    assert_eq!(fixture.queue.enqueued(), 1);
    assert_eq!(
        fixture.logging.detail(SUCCESS_LOGGED_KEY),
        None,
        "a rejected enqueue prevents the success flag"
    );
}

async fn no_logged_events_leave_no_flag() {
    let fixture = Fixture::new();
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert!(fixture.logging.dispatched().is_empty());
    assert_eq!(fixture.queue.enqueued(), 0);
    assert_eq!(fixture.logging.detail(SUCCESS_LOGGED_KEY), None);
}

async fn provider_failure_is_raised_after_logging() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(vec![
        ReceiveStep::Frame(SESSION_UPDATE),
        ReceiveStep::Pending,
    ]);
    fixture.runtime = FakeRuntime::default().with_plan(
        StreamPlan::new(vec![
            Chunk::Text(CONTENT_START),
            Chunk::Text(TEXT_OUTPUT),
            Chunk::Text(CONTENT_END),
        ])
        .failing_receive(3),
    );
    let result = fixture.run(Options::default()).await;
    let error = result.outcome.unwrap_err();
    assert_provider_failure(&error, 424);
    assert_eq!(result.scope.session_committed, Json::Bool(true));
    assert_committed_failure(&result.scope.committed_failure);
    assert_eq!(fixture.logging.dispatched().len(), 1);
    assert_eq!(fixture.logging.dispatched()[0].len(), 1);
    assert_eq!(
        fixture.logging.detail(SUCCESS_LOGGED_KEY),
        Some(Json::Bool(true))
    );
}

fn assert_provider_failure(error: &RealtimeTransportError<FakeError, IoError>, expected: u16) {
    let RealtimeTransportError::Provider(ProviderStreamError::Bedrock {
        status_code: actual,
        ..
    }) = error
    else {
        panic!("a mapped provider failure, not {error:?}");
    };
    assert_eq!(*actual, expected);
}

fn assert_committed_failure(failure: &Json) {
    let Json::Text(text) = failure else {
        panic!("the committed failure is recorded, not {failure:?}");
    };
    assert!(
        text.contains("ModelStreamErrorException: the provider stream broke"),
        "{text}"
    );
}

async fn a_failure_after_the_client_left_is_logged() {
    let mut fixture = Fixture::new();
    fixture.runtime =
        FakeRuntime::default().with_plan(StreamPlan::new(Vec::new()).failing_receive(0));
    let (captured, _guard) = capture();
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    let logs = captured.text();
    assert!(
        logs.contains("stream failed after the client disconnected"),
        "{logs}"
    );
    assert!(logs.contains("ModelStreamErrorException"), "{logs}");
    assert_eq!(result.scope.committed_failure, Json::Null);
    assert_eq!(fixture.runtime.closes(), 1);
}

pub(super) async fn session_close_sequence() {
    close_after_success().await;
    a_connect_failure_has_nothing_to_close().await;
    close_after_open_failure().await;
    close_after_created_failure().await;
    close_after_provider_failure().await;
    close_failure_is_suppressed().await;
    a_normal_provider_end_closes_the_client().await;
    a_cancelled_client_pump_half_closes_the_input().await;
}

async fn a_connect_failure_has_nothing_to_close() {
    let mut fixture = Fixture::new();
    fixture.runtime = FakeRuntime::default().failing_connect(stream_error(
        "the connection failed",
        FakeError::new("ConnectorError", "the fixture refused the connection"),
    ));
    let result = fixture.run(Options::default()).await;
    let error = result.outcome.unwrap_err();
    assert!(matches!(error, RealtimeTransportError::Stream(_)));
    assert_eq!(
        fixture.runtime.closes(),
        0,
        "no client exists before the connection answers"
    );
    assert!(fixture.client.sends().is_empty());
}

async fn close_after_success() {
    let fixture = Fixture::new();
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert_eq!(fixture.runtime.closes(), 1);
    assert_eq!(fixture.client.closes(), vec![(1000, None)]);
}

async fn close_after_open_failure() {
    let mut fixture = Fixture::new();
    fixture.runtime = FakeRuntime::default().failing_invoke(stream_error(
        "the invoke failed",
        FakeError::new("ValidationException", "the fixture refused the model"),
    ));
    let result = fixture.run(Options::default()).await;
    assert!(
        matches!(
            result.outcome.unwrap_err(),
            RealtimeTransportError::Stream(_)
        ),
        "the failed open stays the stream error"
    );
    assert_eq!(fixture.runtime.invocations(), vec![MODEL.to_owned()]);
    assert_eq!(fixture.runtime.closes(), 1);
    assert!(fixture.client.closes().is_empty());
}

async fn close_after_created_failure() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(Vec::new()).failing_send(0);
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_err());
    assert_eq!(fixture.runtime.closes(), 1);
}

async fn close_after_provider_failure() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(vec![ReceiveStep::Pending]);
    fixture.runtime =
        FakeRuntime::default().with_plan(StreamPlan::new(Vec::new()).failing_receive(0));
    let result = fixture.run(Options::default()).await;
    assert!(matches!(
        result.outcome.unwrap_err(),
        RealtimeTransportError::Provider(_)
    ));
    assert_eq!(fixture.runtime.closes(), 1);
    assert!(fixture.runtime.stream().closed());
    assert!(
        fixture.client.closes().is_empty(),
        "a provider failure never forces the client closed"
    );
}

async fn close_failure_is_suppressed() {
    let mut fixture = Fixture::new();
    fixture.runtime = FakeRuntime::default().failing_close();
    let result = fixture.run(Options::default()).await;
    assert!(
        result.outcome.is_ok(),
        "the close failure never replaces the outcome"
    );
    assert_eq!(fixture.runtime.closes(), 1);

    let mut failed = Fixture::new();
    failed.runtime = FakeRuntime::default()
        .failing_close()
        .failing_invoke(stream_error(
            "the invoke failed",
            FakeError::new("SdkError", "the stream never opened"),
        ));
    let failed_result = failed.run(Options::default()).await;
    let error = failed_result.outcome.unwrap_err();
    assert!(matches!(error, RealtimeTransportError::Stream(_)));
}

async fn a_normal_provider_end_closes_the_client() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(vec![ReceiveStep::Pending]);
    fixture.runtime = FakeRuntime::default().with_plan(StreamPlan::new(vec![Chunk::End]));
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert_eq!(fixture.client.closes(), vec![(1000, None)]);
}

async fn a_cancelled_client_pump_half_closes_the_input() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(vec![
        ReceiveStep::Frame(SESSION_UPDATE),
        ReceiveStep::Pending,
    ]);
    fixture.runtime = FakeRuntime::default().with_plan(StreamPlan::new(vec![Chunk::End]));
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert!(fixture.runtime.stream().closed());
    let sent = fixture.runtime.stream().sent();
    let keys: Vec<Option<&str>> = sent.iter().map(event_key).collect();
    assert_eq!(
        keys,
        vec![
            Some("sessionStart"),
            Some("promptStart"),
            Some("promptEnd"),
            Some("sessionEnd"),
        ]
    );
}

pub(super) async fn bridge_completion_race() {
    the_client_failure_wins().await;
    a_provider_failure_maps_to_bedrock().await;
    cancellation_is_not_a_failure().await;
    events_collected_before_the_failure_are_kept().await;
}

async fn the_client_failure_wins() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(vec![ReceiveStep::Frame(SESSION_UPDATE)]);
    fixture.runtime = FakeRuntime::default().with_plan(
        StreamPlan::new(Vec::new())
            .failing_send(0)
            .failing_receive(0),
    );
    let result = fixture.run(Options::default()).await;
    let error = result.outcome.unwrap_err();
    match &error {
        RealtimeTransportError::Provider(ProviderStreamError::Unchanged(failure)) => {
            assert_eq!(failure.class_name, "ClientStreamError");
        }
        other @ (RealtimeTransportError::Committed(_)
        | RealtimeTransportError::Endpoint(_)
        | RealtimeTransportError::Credential(_)
        | RealtimeTransportError::NoCredentials { .. }
        | RealtimeTransportError::Client(_)
        | RealtimeTransportError::SessionCreated { .. }
        | RealtimeTransportError::Stream(_)
        | RealtimeTransportError::Completion(_)
        | RealtimeTransportError::Logging { .. }
        | RealtimeTransportError::Provider(_)) => {
            panic!("the client exception arrives first, not {other:?}");
        }
    }
    assert_eq!(error.status_code(), None);
    assert_eq!(result.scope.committed_failure, Json::Null);
}

async fn a_provider_failure_maps_to_bedrock() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(vec![ReceiveStep::Pending]);
    fixture.runtime =
        FakeRuntime::default().with_plan(StreamPlan::new(Vec::new()).failing_receive(0));
    let result = fixture.run(Options::default()).await;
    let error = result.outcome.unwrap_err();
    assert_eq!(error.status_code(), Some(424));
    assert!(error.to_string().contains("ModelStreamErrorException"));
    assert!(fixture.client.closes().is_empty());
    assert!(fixture.runtime.stream().closed());
}

async fn cancellation_is_not_a_failure() {
    let mut fixture = Fixture::new();
    fixture.runtime = FakeRuntime::default().with_plan(StreamPlan::new(Vec::new()).blocking());
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    assert!(fixture.runtime.stream().closed());
    assert!(
        fixture.client.closes().is_empty(),
        "a cancelled direction is not an end"
    );
    assert!(fixture.logging.dispatched().is_empty());
}

async fn events_collected_before_the_failure_are_kept() {
    let mut fixture = Fixture::new();
    fixture.client = ScriptedClient::scripted(vec![ReceiveStep::Pending]);
    fixture.runtime = FakeRuntime::default().with_plan(
        StreamPlan::new(vec![
            Chunk::Text(CONTENT_START),
            Chunk::Text(TEXT_OUTPUT),
            Chunk::Text(CONTENT_END),
        ])
        .failing_receive(3),
    );
    let result = fixture.run(Options::default()).await;
    assert!(result.outcome.is_err());
    let forwarded = fixture.client.sent_events();
    assert!(
        forwarded
            .iter()
            .any(|event| event_type(event) == Some("response.done")),
        "the forwarded events arrive before the failure"
    );
    assert_eq!(fixture.logging.dispatched()[0].len(), 1);
}

fn record_field<'record>(
    record: &'record IndexMap<String, Json>,
    path: &[&str],
) -> Option<&'record Json> {
    let mut current = record.get(*path.first()?)?;
    for name in &path[1..] {
        current = field(current, name)?;
    }
    Some(current)
}
