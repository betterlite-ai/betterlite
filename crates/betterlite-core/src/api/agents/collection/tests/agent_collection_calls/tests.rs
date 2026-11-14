#![cfg(test)]

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_error::error::details::{ResponseDetails, TimeoutDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::creation::{acreate, create};
use super::errors::{failure as classify, identity_of};
use super::listing::{alist, list};
use super::{
    AgentCollection, AgentCollectionFailure, AgentCollectionHandled, AgentCreation, AgentFailure,
    AgentListing,
};

use self::support::{FixtureConfig, Provider, ProviderRefusal, Recorder};

mod support;

struct Fixture {
    config: FixtureConfig,
    clients: HttpClients,
    logging: LoggingState,
    records: Recorder,
    redactor: Redactor,
    request_timeout: Duration,
}

impl Fixture {
    fn new() -> Self {
        Self {
            config: FixtureConfig::gemini_headers(),
            clients: support::clients(),
            logging: logging_state(),
            records: Recorder::default(),
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                .expect("the default redactor compiles"),
            request_timeout: Duration::from_secs(5),
        }
    }

    fn collection(&mut self) -> AgentCollection<'_, FixtureConfig> {
        AgentCollection {
            config: &self.config,
            clients: &mut self.clients,
            logging: &mut self.logging,
            records: &mut self.records,
            redactor: &self.redactor,
            request_timeout: self.request_timeout,
        }
    }
}

impl AgentFailure for ProviderRefusal {
    fn status_code(&self) -> Option<u16> {
        self.status
    }

    fn message(&self) -> String {
        self.message.clone()
    }
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "fixture".to_owned(),
            call_type: "fixture".to_owned(),
            start_time: moment(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

fn map(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn create_call<'call>(
    name: &'call str,
    kwargs: &'call IndexMap<String, Json>,
) -> AgentCreation<'call> {
    AgentCreation {
        name,
        base_agent: None,
        instructions: None,
        base_environment: None,
        custom_llm_provider: None,
        extra_headers: BTreeMap::new(),
        extra_body: None,
        timeout: None,
        kwargs,
    }
}

fn list_call(kwargs: &IndexMap<String, Json>) -> AgentListing<'_> {
    AgentListing {
        custom_llm_provider: None,
        extra_headers: BTreeMap::new(),
        timeout: None,
        kwargs,
    }
}

fn completed<Output, Pending>(handled: AgentCollectionHandled<Output, Pending>) -> Output {
    match handled {
        AgentCollectionHandled::Done(output) => output,
        AgentCollectionHandled::Pending(_pending) => {
            panic!("the call should have finished synchronously")
        }
    }
}

fn pending<Output, Pending>(handled: AgentCollectionHandled<Output, Pending>) -> Pending {
    match handled {
        AgentCollectionHandled::Pending(pending) => pending,
        AgentCollectionHandled::Done(_output) => panic!("the call should have been pending"),
    }
}

fn failure_of<Output, Pending>(
    outcome: Result<
        AgentCollectionHandled<Output, Pending>,
        AgentCollectionFailure<ProviderRefusal>,
    >,
) -> AgentCollectionFailure<ProviderRefusal> {
    match outcome {
        Ok(_handled) => panic!("the call should have failed"),
        Err(failure) => failure,
    }
}

fn bad_request(class: &ErrorClass) -> &ResponseDetails<400> {
    let ErrorClass::BadRequest(details) = class else {
        panic!("the class should be a bad request");
    };
    details
}

fn authentication(class: &ErrorClass) -> &ResponseDetails<401> {
    let ErrorClass::Authentication(details) = class else {
        panic!("the class should be an authentication error");
    };
    details
}

fn not_found(class: &ErrorClass) -> &ResponseDetails<404> {
    let ErrorClass::NotFound(details) = class else {
        panic!("the class should be a not-found error");
    };
    details
}

fn api_connection(class: &ErrorClass) -> &ErrorContext {
    let ErrorClass::ApiConnection(context) = class else {
        panic!("the class should be an api connection error");
    };
    context
}

fn timed_out(class: &ErrorClass) -> &TimeoutDetails {
    let ErrorClass::Timeout(details) = class else {
        panic!("the class should be a timeout");
    };
    details
}

#[test]
fn create_posts_the_agent_and_returns_the_provider_reply() {
    let provider = Provider::start(200, r#"{"id":"agent-1"}"#, None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("api_key", text("key-1")),
    ]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        extra_headers: headers(&[("x-extra", "yes")]),
        ..create_call("runner", &kwargs)
    };

    let output = completed(create(fixture.collection(), &call).expect("the provider answers"));

    assert_eq!(output, r#"{"id":"agent-1"}"#);
    let sent = provider.sent();
    assert!(sent.starts_with("POST /v1beta/agents HTTP/1.1"), "{sent}");
    assert!(sent.contains("x-extra: yes\r\n"), "{sent}");
    let body = support::sent_body(&sent);
    assert_eq!(
        body.get("name"),
        Some(&JsonValue::Text("runner".to_owned()))
    );
    assert_eq!(
        body.get("api_key"),
        Some(&JsonValue::Text("key-1".to_owned()))
    );
    assert_eq!(fixture.logging.model, "runner");
    assert_eq!(fixture.logging.call_type, "create_agent");
    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
    assert!(fixture.logging.optional_params.is_empty());
    assert!(fixture.records.calls()[0].starts_with("pre|runner|"));
}

#[test]
fn create_forwards_the_base_agent_and_instructions_to_the_provider() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        base_agent: Some("gemini-2.5-flash"),
        instructions: Some("be nice"),
        ..create_call("runner", &kwargs)
    };

    completed(create(fixture.collection(), &call).expect("the provider answers"));

    let body = support::sent_body(&provider.sent());
    assert_eq!(
        body.get("base_agent"),
        Some(&JsonValue::Text("gemini-2.5-flash".to_owned()))
    );
    assert_eq!(
        body.get("instructions"),
        Some(&JsonValue::Text("be nice".to_owned()))
    );
    assert_eq!(
        body.get("name"),
        Some(&JsonValue::Text("runner".to_owned()))
    );
}

#[test]
fn create_keeps_the_base_environment_the_caller_sent() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let environment = Json::Object(map(&[
        ("id", text("env-1")),
        (
            "nested",
            Json::List(vec![number(3), Json::Bool(true), Json::Null]),
        ),
    ]));
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        base_environment: Some(&environment),
        ..create_call("runner", &kwargs)
    };

    completed(create(fixture.collection(), &call).expect("the provider answers"));

    let body = support::sent_body(&provider.sent());
    let Some(JsonValue::Object(sent_environment)) = body.get("base_environment") else {
        panic!("the body carries the environment object");
    };
    assert_eq!(
        sent_environment.get("id"),
        Some(&JsonValue::Text("env-1".to_owned()))
    );
    let Some(JsonValue::Array(items)) = sent_environment.get("nested") else {
        panic!("the body carries the nested list");
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], JsonValue::Number(serde_json::Number::from(3_i32)));
    assert_eq!(items[1], JsonValue::Bool(true));
    assert_eq!(items[2], JsonValue::Null);
}

#[test]
fn create_merges_extra_body_over_the_provider_body() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let labels = map(&[("team", text("core"))]);
    let extra_body = map(&[
        ("name", text("agent-two")),
        ("labels", Json::Object(labels)),
    ]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        extra_body: Some(&extra_body),
        ..create_call("runner", &kwargs)
    };

    completed(create(fixture.collection(), &call).expect("the provider answers"));

    let body = support::sent_body(&provider.sent());
    assert_eq!(
        body.get("name"),
        Some(&JsonValue::Text("agent-two".to_owned()))
    );
    let Some(JsonValue::Object(sent_labels)) = body.get("labels") else {
        panic!("the body carries the labels object");
    };
    assert_eq!(
        sent_labels.get("team"),
        Some(&JsonValue::Text("core".to_owned()))
    );
}

#[test]
fn create_defaults_the_provider_to_gemini() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();

    completed(
        create(fixture.collection(), &create_call("runner", &kwargs)).expect("gemini answers"),
    );

    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
    let body = support::sent_body(&provider.sent());
    assert_eq!(
        body.get("custom_llm_provider"),
        Some(&JsonValue::Text("gemini".to_owned()))
    );
}

#[test]
fn create_takes_the_provider_argument_over_the_kwarg() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("custom_llm_provider", text("openai")),
    ]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        custom_llm_provider: Some("gemini"),
        ..create_call("runner", &kwargs)
    };

    completed(create(fixture.collection(), &call).expect("the argument's provider answers"));

    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
}

#[test]
fn create_takes_an_explicit_empty_provider_as_given() {
    let kwargs = map(&[]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        custom_llm_provider: Some(""),
        ..create_call("runner", &kwargs)
    };

    let failure = failure_of(create(fixture.collection(), &call));

    let details = bad_request(&failure.class);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some(""));
    assert_eq!(
        details.message,
        "Provider '' does not have a native agents API. Use the proxy POST /v1/agents endpoint to store agents locally."
    );
}

#[test]
fn create_rejects_a_provider_kwarg_of_the_wrong_type() {
    let kwargs = map(&[("custom_llm_provider", Json::Bool(true))]);
    let mut fixture = Fixture::new();

    let failure = failure_of(create(
        fixture.collection(),
        &create_call("runner", &kwargs),
    ));

    let details = bad_request(&failure.class);
    assert_eq!(details.model.as_deref(), Some("runner"));
    assert_eq!(details.provider.as_deref(), Some("gemini"));
    assert_eq!(details.body_param.as_deref(), Some("custom_llm_provider"));
    assert_eq!(
        details.message,
        "the custom_llm_provider value must be a string"
    );
    assert_eq!(details.status.0, 400);
}

#[test]
fn create_reports_an_unsupported_provider_before_it_sends_anything() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("custom_llm_provider", text("openai")),
    ]);
    let mut fixture = Fixture::new();

    let failure = failure_of(create(
        fixture.collection(),
        &create_call("runner", &kwargs),
    ));

    let details = bad_request(&failure.class);
    assert_eq!(
        details.message,
        "Provider 'openai' does not have a native agents API. Use the proxy POST /v1/agents endpoint to store agents locally."
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.status.0, 400);
    assert!(provider.requests().is_empty());
    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("openai")
    );
    assert_eq!(fixture.logging.call_type, "create_agent");
}

#[test]
fn create_keeps_running_when_the_async_flag_is_not_true() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("api_key", text("key-1")),
        ("acreate_agent", Json::Bool(false)),
    ]);
    let mut fixture = Fixture::new();

    let output =
        completed(create(fixture.collection(), &create_call("runner", &kwargs)).expect("answers"));

    assert_eq!(output, "{}");
    let body = support::sent_body(&provider.sent());
    assert_eq!(
        body.get("api_key"),
        Some(&JsonValue::Text("key-1".to_owned()))
    );
    assert!(!body.contains_key("acreate_agent"));
}

#[tokio::test]
async fn create_returns_a_pending_call_when_the_async_flag_is_true() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("api_key", text("key-1")),
        ("acreate_agent", Json::Bool(true)),
    ]);
    let mut fixture = Fixture::new();

    let pending_call = pending(
        create(fixture.collection(), &create_call("runner", &kwargs)).expect("the call prepares"),
    );
    let output = pending_call.await.expect("the provider answers");

    assert_eq!(output, "{}");
    let body = support::sent_body(&provider.sent());
    assert_eq!(
        body.get("api_key"),
        Some(&JsonValue::Text("key-1".to_owned()))
    );
    assert!(!body.contains_key("acreate_agent"));
}

#[tokio::test]
async fn acreate_takes_the_default_provider_over_a_kwarg() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("custom_llm_provider", text("openai")),
    ]);
    let mut fixture = Fixture::new();

    acreate(fixture.collection(), create_call("runner", &kwargs))
        .await
        .expect("gemini answers");

    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
}

#[tokio::test]
async fn acreate_awaits_the_create_the_async_entry_starts() {
    let provider = Provider::start(200, r#"{"id":"agent-1"}"#, None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("api_key", text("key-1")),
    ]);
    let mut fixture = Fixture::new();

    let output = acreate(fixture.collection(), create_call("runner", &kwargs))
        .await
        .expect("the provider answers");

    assert_eq!(output, r#"{"id":"agent-1"}"#);
    let body = support::sent_body(&provider.sent());
    assert_eq!(
        body.get("name"),
        Some(&JsonValue::Text("runner".to_owned()))
    );
    assert_eq!(
        body.get("api_key"),
        Some(&JsonValue::Text("key-1".to_owned()))
    );
    assert!(!body.contains_key("acreate_agent"));
    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
    assert_eq!(fixture.logging.call_type, "create_agent");
}

#[test]
fn list_sends_the_page_query_and_returns_the_provider_reply() {
    let provider = Provider::start(200, r#"{"agents":[]}"#, None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("page_size", number(2)),
        ("page_token", text("tok")),
    ]);
    let mut fixture = Fixture::new();

    let output = completed(list(fixture.collection(), &list_call(&kwargs)).expect("answers"));

    assert_eq!(output, r#"{"agents":[]}"#);
    let sent = provider.sent();
    assert!(
        sent.starts_with("GET /v1beta/agents?pageSize=2&pageToken=tok HTTP/1.1"),
        "{sent}"
    );
    assert_eq!(fixture.logging.model, "");
    assert_eq!(fixture.logging.call_type, "list_agents");
    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
    assert!(fixture.logging.optional_params.is_empty());
    assert!(fixture.records.calls()[0].starts_with("pre|list_agents|"));
}

#[test]
fn list_reports_an_unsupported_provider_with_an_empty_model() {
    let kwargs = map(&[("custom_llm_provider", text("openai"))]);
    let mut fixture = Fixture::new();

    let failure = failure_of(list(fixture.collection(), &list_call(&kwargs)));

    let details = bad_request(&failure.class);
    assert_eq!(
        details.message,
        "Provider 'openai' does not have a native agents API. Use the proxy POST /v1/agents endpoint to store agents locally."
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(fixture.logging.model, "");
    assert_eq!(fixture.logging.call_type, "list_agents");
}

#[test]
fn list_uses_the_provider_status_to_classify_a_provider_error() {
    let provider = Provider::start(404, "the agent is gone", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();

    let failure = failure_of(list(fixture.collection(), &list_call(&kwargs)));

    let details = not_found(&failure.class);
    assert_eq!(details.status.0, 404);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("gemini"));
    assert_eq!(
        details.message,
        "gemini: Vertex AI could not find the requested resource"
    );
}

#[test]
fn list_rejects_a_provider_kwarg_of_the_wrong_type() {
    let kwargs = map(&[("custom_llm_provider", Json::Bool(true))]);
    let mut fixture = Fixture::new();

    let failure = failure_of(list(fixture.collection(), &list_call(&kwargs)));

    let details = bad_request(&failure.class);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("gemini"));
    assert_eq!(details.body_param.as_deref(), Some("custom_llm_provider"));
}

#[test]
fn list_forwards_extra_headers_to_the_provider() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();
    let call = AgentListing {
        extra_headers: headers(&[("x-extra", "yes")]),
        ..list_call(&kwargs)
    };

    completed(list(fixture.collection(), &call).expect("the provider answers"));

    let sent = provider.sent();
    assert!(sent.starts_with("GET /v1beta/agents HTTP/1.1"), "{sent}");
    assert!(sent.contains("x-extra: yes\r\n"), "{sent}");
}

#[test]
fn list_keeps_running_when_the_async_flag_is_not_true() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("alist_agents", Json::Bool(false)),
    ]);
    let mut fixture = Fixture::new();

    let output = completed(list(fixture.collection(), &list_call(&kwargs)).expect("answers"));

    assert_eq!(output, "{}");
    let sent = provider.sent();
    assert!(sent.starts_with("GET /v1beta/agents HTTP/1.1"), "{sent}");
    assert!(!sent.contains("alist_agents"));
}

#[tokio::test]
async fn list_returns_a_pending_call_when_the_async_flag_is_true() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("alist_agents", Json::Bool(true)),
    ]);
    let mut fixture = Fixture::new();

    let pending_call =
        pending(list(fixture.collection(), &list_call(&kwargs)).expect("the call prepares"));
    let output = pending_call.await.expect("the provider answers");

    assert_eq!(output, "{}");
    let sent = provider.sent();
    assert!(sent.starts_with("GET /v1beta/agents HTTP/1.1"), "{sent}");
    assert!(!sent.contains("alist_agents"));
}

#[tokio::test]
async fn alist_awaits_the_list_the_async_entry_starts() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("page_size", number(2)),
    ]);
    let mut fixture = Fixture::new();

    let output = alist(fixture.collection(), list_call(&kwargs))
        .await
        .expect("the provider answers");

    assert_eq!(output, "{}");
    assert!(
        provider
            .sent()
            .starts_with("GET /v1beta/agents?pageSize=2 HTTP/1.1")
    );
    assert_eq!(fixture.logging.model, "");
    assert_eq!(fixture.logging.call_type, "list_agents");
    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
}

#[tokio::test]
async fn alist_takes_the_default_provider_over_a_kwarg() {
    let provider = Provider::start(200, "{}", None);
    let kwargs = map(&[
        ("api_base", text(&provider.url())),
        ("custom_llm_provider", text("openai")),
    ]);
    let mut fixture = Fixture::new();

    alist(fixture.collection(), list_call(&kwargs))
        .await
        .expect("gemini answers");

    assert_eq!(
        fixture.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
}

#[tokio::test]
async fn alist_uses_the_provider_status_to_classify_a_provider_error() {
    let provider = Provider::start(404, "the agent is gone", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();

    let failure = alist(fixture.collection(), list_call(&kwargs))
        .await
        .expect_err("the provider refuses");

    let details = not_found(&failure.class);
    assert_eq!(details.status.0, 404);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("gemini"));
}

// a list request carries no deadline of its own, so this feeds the timeout a failed GET would report into the same classifier
#[test]
fn list_keeps_a_transport_timeout_as_a_connection_error() {
    let provider = Provider::start(200, "{}", Some(Duration::from_millis(300)));
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        timeout: Some(Duration::from_millis(20)),
        ..create_call("runner", &kwargs)
    };
    let transport = failure_of(create(fixture.collection(), &call)).error;

    let listed = classify(
        transport,
        &identity_of("", "gemini", false),
        &fixture.redactor,
    );

    let context = api_connection(&listed.class);
    assert_eq!(listed.class.metadata().status_code, 500);
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some("gemini"));
}

#[test]
fn create_uses_the_provider_status_to_classify_a_provider_error() {
    let provider = Provider::start(404, "the agent is gone", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();

    let failure = failure_of(create(
        fixture.collection(),
        &create_call("runner", &kwargs),
    ));

    let details = not_found(&failure.class);
    assert_eq!(details.status.0, 404);
    assert_eq!(details.model.as_deref(), Some("runner"));
    assert_eq!(details.provider.as_deref(), Some("gemini"));
    assert_eq!(
        details.message,
        "gemini: Vertex AI could not find the requested resource"
    );
    assert!(
        fixture
            .records
            .calls()
            .iter()
            .all(|recorded| !recorded.starts_with("post|"))
    );
}

#[test]
fn create_uses_the_provider_message_to_classify_a_provider_error() {
    let provider = Provider::start(400, "API key not valid.", None);
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();

    let failure = failure_of(create(
        fixture.collection(),
        &create_call("runner", &kwargs),
    ));

    let details = authentication(&failure.class);
    assert_eq!(details.status.0, 401);
    assert_eq!(
        details.message,
        "gemini: the Vertex AI API key is not valid"
    );
}

#[test]
fn create_reports_a_transport_timeout_as_a_timeout_error() {
    let provider = Provider::start(200, "{}", Some(Duration::from_millis(300)));
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        timeout: Some(Duration::from_millis(20)),
        ..create_call("runner", &kwargs)
    };

    let failure = failure_of(create(fixture.collection(), &call));

    let details = timed_out(&failure.class);
    assert_eq!(details.original_status.0, 408);
    assert_eq!(details.context.message, "the request to gemini timed out");
    assert_eq!(details.context.model.as_deref(), Some("runner"));
    assert_eq!(details.context.provider.as_deref(), Some("gemini"));
    assert!(
        fixture
            .records
            .calls()
            .iter()
            .all(|recorded| !recorded.starts_with("post|"))
    );
}

#[tokio::test]
async fn acreate_reports_a_transport_timeout_as_a_timeout_error() {
    let provider = Provider::start(200, "{}", Some(Duration::from_millis(300)));
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();
    let call = AgentCreation {
        timeout: Some(Duration::from_millis(20)),
        ..create_call("runner", &kwargs)
    };

    let failure = acreate(fixture.collection(), call)
        .await
        .expect_err("the provider does not answer in time");

    let details = timed_out(&failure.class);
    assert_eq!(details.original_status.0, 408);
    assert_eq!(details.context.model.as_deref(), Some("runner"));
}

#[test]
fn create_reports_a_refused_connection_as_an_api_connection_error() {
    let closed = TcpListener::bind("127.0.0.1:0").expect("the fixture binds");
    let address = closed.local_addr().expect("the address is readable");
    drop(closed);
    let kwargs = map(&[("api_base", text(&format!("http://{address}")))]);
    let mut fixture = Fixture::new();

    let failure = failure_of(create(
        fixture.collection(),
        &create_call("runner", &kwargs),
    ));

    let context = api_connection(&failure.class);
    assert_eq!(failure.class.metadata().status_code, 500);
    assert_eq!(
        context.message,
        "GeminiException - the request to the provider failed"
    );
    assert_eq!(context.model.as_deref(), Some("runner"));
    assert_eq!(context.provider.as_deref(), Some("gemini"));
}

#[test]
fn create_sends_the_collection_request_timeout() {
    let provider = Provider::start(200, "{}", Some(Duration::from_millis(300)));
    let kwargs = map(&[("api_base", text(&provider.url()))]);
    let mut fixture = Fixture::new();
    fixture.request_timeout = Duration::from_millis(20);

    let failure = failure_of(create(
        fixture.collection(),
        &create_call("runner", &kwargs),
    ));

    let details = timed_out(&failure.class);
    assert_eq!(details.original_status.0, 408);
    assert_eq!(details.context.model.as_deref(), Some("runner"));
}
