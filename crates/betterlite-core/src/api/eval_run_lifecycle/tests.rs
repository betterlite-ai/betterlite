#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::Serving;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rstest::rstest;

use self::support::{
    CALL_ID, CANCELLED_FIXTURE, DELETED_FIXTURE, EVAL_ID, Fake, LogEnv, OPENAI_BASE,
    REQUEST_TIMEOUT, RUN_FIXTURE, RUN_ID, RecordingConfig, body_json, headers, json, json_server,
    keywords, object, provider_error, proxied_client, state, url,
};
use crate::api::eval_run_lifecycle::error::EvalRunError;
use crate::api::eval_run_lifecycle::handlers::{EvalRunHandlers, HttpRunHandlers};
use crate::api::eval_run_lifecycle::{
    EvalRunDeps, EvalRunRequest, acancel_run, adelete_run, aget_run, cancel_run, delete_run,
    get_run,
};

fn test_redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

fn run_deps<'deps, Handler>(
    handler: &'deps Handler,
    config: &'deps RecordingConfig,
    redactor: &'deps Redactor,
) -> EvalRunDeps<'deps, Handler>
where
    Handler: EvalRunHandlers,
{
    EvalRunDeps {
        handler,
        config,
        redactor,
        request_timeout: REQUEST_TIMEOUT,
    }
}

fn base_request(kwargs: &IndexMap<String, Json>) -> EvalRunRequest<'_> {
    EvalRunRequest {
        eval_id: EVAL_ID,
        run_id: RUN_ID,
        extra_headers: None,
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
        kwargs,
    }
}

fn auth_kwargs() -> IndexMap<String, Json> {
    keywords(&[
        ("api_key", Json::Text("sk-evals".to_owned())),
        ("litellm_call_id", Json::Text(CALL_ID.to_owned())),
    ])
}

// The operation one wire case drives through the real handler
#[derive(Clone, Copy)]
enum Wire {
    Get,
    Cancel,
    Delete,
}

impl Wire {
    // Runs this operation through the production handler and hands back the id it decoded
    async fn perform(
        self,
        deps: &EvalRunDeps<'_, HttpRunHandlers<'_>>,
        request: &EvalRunRequest<'_>,
        logging: &mut LoggingState,
    ) -> String {
        match self {
            Self::Get => {
                aget_run(deps, request, logging)
                    .await
                    .expect("the get succeeds")
                    .id
            }
            Self::Cancel => {
                acancel_run(deps, request, logging)
                    .await
                    .expect("the cancel succeeds")
                    .id
            }
            Self::Delete => {
                adelete_run(deps, request, logging)
                    .await
                    .expect("the delete succeeds")
                    .run_id
            }
        }
    }
}

// Drives one operation over the fixture and hands back the id it decoded and the request the fixture saw
async fn wire_request(
    server: Serving,
    operation: Wire,
    config: &RecordingConfig,
) -> (String, Request) {
    let redactor = test_redactor();
    let client = proxied_client(TransportLimits::DEFAULT);
    let env = LogEnv::default();
    let pre_post = env.pre_post();
    let handlers = HttpRunHandlers {
        client: &client,
        env: &pre_post,
    };
    let deps = run_deps(&handlers, config, &redactor);
    let mut kwargs = auth_kwargs();
    kwargs.insert("api_base".to_owned(), Json::Text(url(&server, "")));
    let request = base_request(&kwargs);
    let mut logging = state("wire");
    let id = operation.perform(&deps, &request, &mut logging).await;
    (id, server.finish().expect("the fixture receives the call"))
}

// Each sync operation returns its own response contract with the run id it answered
#[test]
fn eval_run_lifecycle_exports_return_each_operation_response() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = auth_kwargs();
    let request = base_request(&kwargs);
    let mut logging = state("get_run");
    let run = get_run(&deps, &request, &mut logging).expect("the get succeeds");
    let cancelled = cancel_run(&deps, &request, &mut logging).expect("the cancel succeeds");
    let deleted = delete_run(&deps, &request, &mut logging).expect("the delete succeeds");
    assert_eq!(run.id, RUN_ID);
    assert_eq!(run.eval_id, EVAL_ID);
    assert_eq!(cancelled.id, RUN_ID);
    assert_eq!(deleted.run_id, RUN_ID);
    assert_eq!(
        fake.calls()
            .iter()
            .map(|call| call.action)
            .collect::<Vec<_>>(),
        ["get", "cancel", "delete"]
    );
}

// The request carries both ids in the URL, the default base, the validated headers and no query string
#[test]
fn eval_run_lifecycle_exports_send_the_ids_headers_and_timeout() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let extra = headers(&[("x-test", "yes")]);
    let query = object(r#"{"trace":true}"#);
    let kwargs = auth_kwargs();
    let request = EvalRunRequest {
        extra_headers: Some(&extra),
        extra_query: Some(&query),
        ..base_request(&kwargs)
    };
    let mut logging = state("get_run");
    get_run(&deps, &request, &mut logging).expect("the get succeeds");
    let seen = fake.calls();
    assert_eq!(
        seen[0].url,
        format!("{OPENAI_BASE}/v1/evals/{EVAL_ID}/runs/{RUN_ID}")
    );
    assert_eq!(
        seen[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-evals")
    );
    assert_eq!(
        seen[0].headers.get("x-test").map(String::as_str),
        Some("yes")
    );
    assert!(!seen[0].url.contains('?'));
    assert!(!seen[0].is_async);
    assert_eq!(seen[0].timeout, Some(REQUEST_TIMEOUT));
}

// Both ids land in optional_params and the call details, with the provider and the caller's call id
#[test]
fn eval_run_lifecycle_exports_record_both_ids_in_logging() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = auth_kwargs();
    let mut logging = state("get_run");
    get_run(&deps, &base_request(&kwargs), &mut logging).expect("the get succeeds");
    let eval = Json::Text(EVAL_ID.to_owned());
    let run = Json::Text(RUN_ID.to_owned());
    assert_eq!(logging.optional_params.get("eval_id"), Some(&eval));
    assert_eq!(logging.optional_params.get("run_id"), Some(&run));
    assert_eq!(logging.model_call_details.get("eval_id"), Some(&eval));
    assert_eq!(logging.model_call_details.get("run_id"), Some(&run));
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        logging.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(CALL_ID.to_owned()))
    );
}

// A metadata entry in kwargs reaches litellm_params and the call details, as update_from_kwargs records it
#[test]
fn eval_run_lifecycle_exports_forward_the_kwargs_metadata() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let mut kwargs = auth_kwargs();
    let metadata = IndexMap::from([(
        "user_api_key_alias".to_owned(),
        Json::Text("alias".to_owned()),
    )]);
    kwargs.insert("metadata".to_owned(), Json::Object(metadata.clone()));
    let mut logging = state("get_run");
    get_run(&deps, &base_request(&kwargs), &mut logging).expect("the get succeeds");
    let forwarded = Json::Object(metadata);
    assert_eq!(logging.litellm_params.get("metadata"), Some(&forwarded));
    let Some(Json::Object(recorded)) = logging.model_call_details.get("litellm_params") else {
        panic!("the call details carry the litellm params");
    };
    assert_eq!(recorded.get("metadata"), Some(&forwarded));
}

// litellm_call_id keeps its None default when kwargs never had one
#[test]
fn eval_run_lifecycle_exports_default_the_logged_call_id() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let bare = keywords(&[("api_key", Json::Text("sk-evals".to_owned()))]);
    let mut logging = state("get_run");
    get_run(&deps, &base_request(&bare), &mut logging).expect("the get succeeds");
    assert_eq!(
        logging.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}

// api_base reaches the transform as given, while a null one takes DEFAULT_OPENAI_API_BASE and an empty string stays empty
#[rstest]
#[case::given(
    Json::Text(String::from("https://gateway.example")),
    "https://gateway.example"
)]
#[case::empty(Json::Text(String::new()), "")]
#[case::null(Json::Null, OPENAI_BASE)]
fn eval_run_lifecycle_exports_use_the_given_api_base(#[case] value: Json, #[case] base: &str) {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = keywords(&[("api_base", value)]);
    let mut logging = state("get_run");
    get_run(&deps, &base_request(&kwargs), &mut logging).expect("the get succeeds");
    let seen = fake.calls();
    assert_eq!(
        seen[0].url,
        format!("{base}/v1/evals/{EVAL_ID}/runs/{RUN_ID}")
    );
}

// A known provider without an evals config raises LiteLLM's ValueError text for each action
#[test]
fn eval_run_lifecycle_unsupported_known_provider_names_the_action() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = auth_kwargs();
    let azure = EvalRunRequest {
        custom_llm_provider: Some("azure"),
        ..base_request(&kwargs)
    };
    let mut logging = state("get_run");
    let get_failure = get_run(&deps, &azure, &mut logging).expect_err("azure has no evals config");
    let cancel_failure =
        cancel_run(&deps, &azure, &mut logging).expect_err("azure has no evals config");
    let delete_failure =
        delete_run(&deps, &azure, &mut logging).expect_err("azure has no evals config");
    assert_eq!(
        get_failure.error.to_string(),
        "GET run is not supported for azure"
    );
    assert_eq!(
        cancel_failure.error.to_string(),
        "CANCEL run is not supported for azure"
    );
    assert_eq!(
        delete_failure.error.to_string(),
        "DELETE run is not supported for azure"
    );
    let ErrorClass::BadRequest(get_details) = &get_failure.class else {
        panic!("a bad request, got {:?}", get_failure.class);
    };
    let ErrorClass::BadRequest(cancel_details) = &cancel_failure.class else {
        panic!("a bad request, got {:?}", cancel_failure.class);
    };
    let ErrorClass::BadRequest(delete_details) = &delete_failure.class else {
        panic!("a bad request, got {:?}", delete_failure.class);
    };
    assert_eq!(
        [
            get_details.status.0,
            cancel_details.status.0,
            delete_details.status.0
        ],
        [400, 400, 400]
    );
    assert_eq!(
        [
            get_details.model.as_deref(),
            cancel_details.model.as_deref(),
            delete_details.model.as_deref()
        ],
        [Some(""), Some(""), Some("")]
    );
    assert_eq!(
        [
            get_details.provider.as_deref(),
            cancel_details.provider.as_deref(),
            delete_details.provider.as_deref()
        ],
        [Some("azure"), Some("azure"), Some("azure")]
    );
    assert!(fake.calls().is_empty());
}

// A name outside the provider set raises the ValueError litellm.LlmProviders raises
#[test]
fn eval_run_lifecycle_unsupported_unknown_provider_raises_the_lookup_error() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = auth_kwargs();
    let unknown = EvalRunRequest {
        custom_llm_provider: Some("not-a-provider"),
        ..base_request(&kwargs)
    };
    let mut logging = state("get_run");
    let get_failure =
        get_run(&deps, &unknown, &mut logging).expect_err("the name is outside the set");
    let cancel_failure =
        cancel_run(&deps, &unknown, &mut logging).expect_err("the name is outside the set");
    let delete_failure =
        delete_run(&deps, &unknown, &mut logging).expect_err("the name is outside the set");
    assert_eq!(
        get_failure.error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
    assert_eq!(
        cancel_failure.error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
    assert_eq!(
        delete_failure.error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
    let ErrorClass::BadRequest(get_details) = &get_failure.class else {
        panic!("a bad request, got {:?}", get_failure.class);
    };
    let ErrorClass::BadRequest(cancel_details) = &cancel_failure.class else {
        panic!("a bad request, got {:?}", cancel_failure.class);
    };
    let ErrorClass::BadRequest(delete_details) = &delete_failure.class else {
        panic!("a bad request, got {:?}", delete_failure.class);
    };
    assert_eq!(
        [
            get_details.status.0,
            cancel_details.status.0,
            delete_details.status.0
        ],
        [400, 400, 400]
    );
    assert_eq!(
        [
            get_details.model.as_deref(),
            cancel_details.model.as_deref(),
            delete_details.model.as_deref()
        ],
        [Some(""), Some(""), Some("")]
    );
    assert_eq!(
        [
            get_details.provider.as_deref(),
            cancel_details.provider.as_deref(),
            delete_details.provider.as_deref()
        ],
        [
            Some("not-a-provider"),
            Some("not-a-provider"),
            Some("not-a-provider")
        ]
    );
    assert!(fake.calls().is_empty());
}

// A provider answer keeps the status exception_type maps it to
#[test]
fn eval_run_lifecycle_error_keeps_the_provider_status() {
    let fake = Fake::failing(provider_error(429, "slow down"));
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = auth_kwargs();
    let mut logging = state("cancel_run");
    let failure = cancel_run(&deps, &base_request(&kwargs), &mut logging)
        .expect_err("the handler answers with a provider error");
    assert!(matches!(&failure.error, EvalRunError::Handler(_)));
    assert!(matches!(&failure.class, ErrorClass::RateLimit(_)));
    assert_eq!(failure.class.metadata().status_code, 429);
}

// An api_base of the wrong type is a 400 that names the field
#[test]
fn eval_run_lifecycle_error_refuses_a_non_string_api_base() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let wrong = keywords(&[("api_base", Json::Bool(true))]);
    let mut logging = state("get_run");
    let failure = get_run(&deps, &base_request(&wrong), &mut logging)
        .expect_err("a bool api_base is refused");
    assert!(matches!(&failure.error, EvalRunError::ApiBase));
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a bad api_base is a bad request, not {:?}", failure.class);
    };
    assert_eq!(details.message, "api_base must be a string");
    assert_eq!(details.body_param.as_deref(), Some("api_base"));
    assert_eq!(failure.class.metadata().status_code, 400);
    assert!(fake.calls().is_empty());
}

// validate_environment's missing key reaches exception_type as the ValueError LiteLLM raises
#[test]
fn eval_run_lifecycle_error_missing_api_key_raises_the_value_error() {
    let fake = Fake::default();
    let config = RecordingConfig::without_key();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = keywords(&[]);
    let mut logging = state("get_run");
    let failure = get_run(&deps, &base_request(&kwargs), &mut logging)
        .expect_err("no api key is a refused call");
    assert_eq!(
        failure.error.to_string(),
        "OPENAI_API_KEY is required for Evals API"
    );
    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(failure.class.metadata().status_code, 500);
    assert!(fake.calls().is_empty());
}

// The async twins mark the call in kwargs before GenericLiteLLMParams reads it, then await the handler
#[tokio::test]
async fn eval_run_lifecycle_async_variants_await_the_handler_with_the_flag() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let mut kwargs = auth_kwargs();
    kwargs.insert("aget_run".to_owned(), Json::Bool(false));
    kwargs.insert("trace_extra".to_owned(), Json::Bool(true));
    let request = base_request(&kwargs);
    let mut logging = state("aget_run");
    let run = aget_run(&deps, &request, &mut logging)
        .await
        .expect("the async get succeeds");
    let after_get = config.params().expect("the get params were validated");
    let cancelled = acancel_run(&deps, &request, &mut logging)
        .await
        .expect("the async cancel succeeds");
    let after_cancel = config.params().expect("the cancel params were validated");
    let deleted = adelete_run(&deps, &request, &mut logging)
        .await
        .expect("the async delete succeeds");
    let after_delete = config.params().expect("the delete params were validated");
    assert_eq!(
        (
            run.id.as_str(),
            cancelled.id.as_str(),
            deleted.run_id.as_str()
        ),
        (RUN_ID, RUN_ID, RUN_ID)
    );
    let seen = fake.calls();
    assert!(seen.iter().all(|call| call.is_async));
    assert_eq!(
        seen[0].url,
        format!("{OPENAI_BASE}/v1/evals/{EVAL_ID}/runs/{RUN_ID}")
    );
    // the wrapper's own flag is popped for each action, so it never reaches the params
    assert!(!after_get.contains_key("aget_run"));
    assert!(!after_cancel.contains_key("acancel_run"));
    assert!(!after_delete.contains_key("adelete_run"));
    assert!(after_delete.contains_key("api_key"));
    assert!(after_delete.contains_key("trace_extra"));
}

// An error the async handler raises is mapped once, with the provider the call resolved
#[tokio::test]
async fn eval_run_lifecycle_async_maps_a_handler_error_once() {
    let failing = Fake::failing(provider_error(500, "upstream broke"));
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&failing, &config, &redactor);
    let kwargs = auth_kwargs();
    let mut logging = state("adelete_run");
    let failure = adelete_run(&deps, &base_request(&kwargs), &mut logging)
        .await
        .expect_err("the handler fails");
    assert!(matches!(&failure.error, EvalRunError::Handler(_)));
    assert!(matches!(&failure.class, ErrorClass::InternalServer(_)));
    assert_eq!(failure.class.metadata().provider.as_deref(), Some("openai"));
    assert_eq!(failure.class.metadata().status_code, 500);
}

// A blocking operation called inside a runtime returns the typed refusal instead of blocking that runtime
#[tokio::test]
async fn eval_run_lifecycle_async_refuses_a_blocking_call_inside_a_runtime() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = auth_kwargs();
    let mut logging = state("get_run");
    let failure = get_run(&deps, &base_request(&kwargs), &mut logging)
        .expect_err("a blocking call cannot drive a runtime inside one");
    assert!(matches!(&failure.error, EvalRunError::SyncInRuntime));
    assert_eq!(failure.class.metadata().status_code, 500);
}

// A supplied timeout is used as given, even zero, and an absent one takes the request timeout
#[test]
fn eval_run_lifecycle_timeout_uses_the_supplied_value_or_the_request_timeout() {
    let fake = Fake::default();
    let config = RecordingConfig::default();
    let redactor = test_redactor();
    let deps = run_deps(&fake, &config, &redactor);
    let kwargs = auth_kwargs();
    let mut logging = state("get_run");
    get_run(&deps, &base_request(&kwargs), &mut logging).expect("the default get succeeds");
    let patient = EvalRunRequest {
        timeout: Some(Duration::from_secs(7)),
        ..base_request(&kwargs)
    };
    cancel_run(&deps, &patient, &mut logging).expect("the explicit cancel succeeds");
    let immediate = EvalRunRequest {
        timeout: Some(Duration::ZERO),
        ..base_request(&kwargs)
    };
    delete_run(&deps, &immediate, &mut logging).expect("the zero delete succeeds");
    let seen = fake.calls();
    assert_eq!(seen[0].timeout, Some(REQUEST_TIMEOUT));
    assert_eq!(seen[1].timeout, Some(Duration::from_secs(7)));
    assert_eq!(seen[2].timeout, Some(Duration::ZERO));
}

// The cancel transform's marker body is dropped before the transport, which posts the literal {}
#[tokio::test]
async fn eval_run_lifecycle_body_cancel_sends_an_empty_object() {
    let config = RecordingConfig::default();
    let server = json_server("200 OK", CANCELLED_FIXTURE, Duration::ZERO);
    let (id, request) = wire_request(server, Wire::Cancel, &config).await;
    assert_eq!(id, RUN_ID);
    assert_eq!(
        request.line,
        "POST /v1/evals/eval_1/runs/run_1/cancel HTTP/1.1"
    );
    assert_eq!(body_json(&request), json("{}"));
    assert_eq!(config.transforms(), ["cancel_run_request"]);
}

// The delete transform's marker body is dropped before the transport, which sends no body
#[tokio::test]
async fn eval_run_lifecycle_body_delete_sends_no_body() {
    let config = RecordingConfig::default();
    let server = json_server("200 OK", DELETED_FIXTURE, Duration::ZERO);
    let (id, request) = wire_request(server, Wire::Delete, &config).await;
    assert_eq!(id, RUN_ID);
    assert_eq!(request.line, "DELETE /v1/evals/eval_1/runs/run_1 HTTP/1.1");
    assert!(request.body.is_empty());
    assert_eq!(config.transforms(), ["delete_run_request"]);
}

// The get transform returns no body, so the GET carries none and the reply decodes into a Run
#[tokio::test]
async fn eval_run_lifecycle_body_get_sends_no_body() {
    let config = RecordingConfig::default();
    let server = json_server("200 OK", RUN_FIXTURE, Duration::ZERO);
    let (id, request) = wire_request(server, Wire::Get, &config).await;
    assert_eq!(id, RUN_ID);
    assert_eq!(request.line, "GET /v1/evals/eval_1/runs/run_1 HTTP/1.1");
    assert!(request.body.is_empty());
    assert!(config.transforms().is_empty());
}
