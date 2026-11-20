#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::api::rerank_provider_plan::{Environment, RerankSettings};
use betterlite_error::dispatch::RawException;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::call::{RerankCall, ReturnDocuments};
use super::dispatch::{
    Headers, OptionsRequest, Params, RerankOperations, SendRequest, provider_failure, value_failure,
};
use super::plan::{CatalogIdentity, RerankIdentity, RerankPlan, rerank};
use super::selection::SelectedConfig;

mod families;

#[derive(Debug, Clone)]
struct OptionsSeen {
    config: SelectedConfig,
    model: String,
    drop_params: bool,
    query: String,
    documents: Vec<RerankDocument>,
    custom_llm_provider: Option<String>,
    top_n: Option<i64>,
    rank_fields: Option<Vec<String>>,
    return_documents: ReturnDocuments,
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
    instruction: Option<String>,
    non_default_keys: Vec<String>,
}

#[derive(Debug, Clone)]
struct SendSeen {
    config: SelectedConfig,
    provider: String,
    model: String,
    query: String,
    documents: Vec<RerankDocument>,
    optional_params: Params,
    litellm_params: Params,
    generic_params: Params,
    api_key: Option<String>,
    api_base: Option<String>,
    headers: Headers,
    timeout_seconds: Option<f64>,
    is_async: bool,
    top_n: Option<i64>,
    rank_fields: Option<Vec<String>>,
    return_documents: ReturnDocuments,
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
    instruction: Option<String>,
}

#[derive(Default)]
struct FakeOps {
    options: Mutex<Vec<OptionsSeen>>,
    sends: Mutex<Vec<SendSeen>>,
    mapping_failure: Option<String>,
    send_failure: Option<(u16, String)>,
}

impl FakeOps {
    fn failing_send(status: u16, message: &str) -> Self {
        Self {
            options: Mutex::new(Vec::new()),
            sends: Mutex::new(Vec::new()),
            mapping_failure: None,
            send_failure: Some((status, message.to_owned())),
        }
    }

    fn options(&self) -> Vec<OptionsSeen> {
        self.options
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn sends(&self) -> Vec<SendSeen> {
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RerankOperations for FakeOps {
    fn map_options(&self, request: OptionsRequest<'_>) -> Result<Params, RawException> {
        if let Some(message) = &self.mapping_failure {
            return Err(value_failure(
                message,
                request.model,
                request.custom_llm_provider.unwrap_or_default(),
            ));
        }
        self.options
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(OptionsSeen {
                config: request.config,
                model: request.model.to_owned(),
                drop_params: request.drop_params,
                query: request.query.to_owned(),
                documents: request.documents.to_vec(),
                custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
                top_n: request.top_n,
                rank_fields: request.rank_fields.map(<[String]>::to_vec),
                return_documents: request.return_documents,
                max_chunks_per_doc: request.max_chunks_per_doc,
                max_tokens_per_doc: request.max_tokens_per_doc,
                instruction: request.instruction.map(str::to_owned),
                non_default_keys: request
                    .non_default_params
                    .keys()
                    .map(String::clone)
                    .collect(),
            });
        let mut params = Params::new();
        params.insert("query".to_owned(), Json::Text(request.query.to_owned()));
        params.insert(
            "documents".to_owned(),
            Json::List(
                request
                    .documents
                    .iter()
                    .map(RerankDocument::as_json)
                    .collect(),
            ),
        );
        if let Some(value) = request.return_documents.mapped() {
            params.insert("return_documents".to_owned(), Json::Bool(value));
        }
        if let Some(top_n) = request.top_n {
            params.insert("top_n".to_owned(), Json::Number(Number::from(top_n)));
        }
        Ok(params)
    }

    async fn send(
        &self,
        request: SendRequest<'_>,
        _logging: &mut LoggingState,
    ) -> Result<RerankResponse, RawException> {
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SendSeen {
                config: request.config,
                provider: request.provider.to_owned(),
                model: request.model.to_owned(),
                query: request.query.to_owned(),
                documents: request.documents.to_vec(),
                optional_params: request.optional_params.clone(),
                litellm_params: request.litellm_params.clone(),
                generic_params: request.generic_params.clone(),
                api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                api_base: request.api_base.map(str::to_owned),
                headers: request.headers.clone(),
                timeout_seconds: request.timeout_seconds,
                is_async: request.is_async,
                top_n: request.top_n,
                rank_fields: request.rank_fields.map(<[String]>::to_vec),
                return_documents: request.return_documents,
                max_chunks_per_doc: request.max_chunks_per_doc,
                max_tokens_per_doc: request.max_tokens_per_doc,
                instruction: request.instruction.map(str::to_owned),
            });
        if let Some((status, message)) = &self.send_failure {
            return Err(provider_failure(
                *status,
                message,
                request.model,
                request.provider,
                "BaseLLMException",
            ));
        }
        Ok(reply())
    }
}

struct FakeIdentity {
    resolved: Option<ResolvedModel>,
}

impl FakeIdentity {
    fn new(model: &str, provider: ProviderIdentity, key: Option<&str>, base: Option<&str>) -> Self {
        Self {
            resolved: Some(ResolvedModel {
                model: model.to_owned(),
                provider,
                api_base: base.map(str::to_owned),
                api_key: key.map(|key| SecretString::from(key.to_owned())),
            }),
        }
    }

    fn missing() -> Self {
        Self { resolved: None }
    }
}

impl RerankIdentity for FakeIdentity {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError> {
        self.resolved
            .clone()
            .ok_or_else(|| ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            })
    }
}

fn reply() -> RerankResponse {
    serde_json::from_str(
        r#"{"id":"r","results":[{"index":1,"relevance_score":0.9},{"index":0,"relevance_score":0.1}]}"#,
    )
    .expect("the fixture is a rerank response")
}

fn logging_state() -> LoggingState {
    let moment =
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339");
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "rerank-v1".to_owned(),
            call_type: "rerank".to_owned(),
            start_time: moment,
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

fn defaults() -> RerankSettings {
    RerankSettings {
        api_key: None,
        api_base: None,
        headers: None,
        togetherai_api_key: None,
        drop_params: false,
        environment: Environment::default(),
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn call(provider: &str) -> RerankCall {
    RerankCall {
        model: String::from("rerank-v1"),
        query: String::from("q"),
        documents: vec![
            RerankDocument::Text(String::from("a")),
            RerankDocument::Text(String::from("b")),
        ],
        custom_llm_provider: Some(provider.to_owned()),
        top_n: None,
        rank_fields: None,
        return_documents: ReturnDocuments::Default,
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        kwargs: IndexMap::new(),
    }
}

fn with(kwargs: &[(&str, Json)]) -> IndexMap<String, Json> {
    kwargs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn dispatch<Identity: RerankIdentity>(
    identity: &Identity,
    ops: &FakeOps,
    settings: &RerankSettings,
    environment: &BTreeMap<String, String>,
    call: RerankCall,
    logging: &mut LoggingState,
) -> Result<RerankResponse, super::plan::RerankException> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles");
    let plan = RerankPlan {
        identity,
        operations: ops,
        settings,
        environment,
        redactor: &redactor,
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
        .block_on(rerank(&plan, call, logging))
}

#[test]
fn rerank_defaults_and_options() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("dynamic-key"),
        Some("https://dynamic.test/v1"),
    );
    let mut logging = logging_state();
    let response = dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call("cohere"),
        &mut logging,
    )
    .expect("the default call dispatches");
    let results: Vec<i64> = response
        .results
        .expect("the fixture carries results")
        .into_iter()
        .map(|result| result.index)
        .collect();
    assert_eq!(results, vec![1_i64, 0_i64]);
    let options = ops.options();
    assert_eq!(options[0].return_documents, ReturnDocuments::Default);
    assert_eq!(options[0].query, "q");
    assert_eq!(options[0].documents.len(), 2);
    assert!(options[0].non_default_keys.is_empty());
    let sends = ops.sends();
    assert_eq!(sends[0].return_documents, ReturnDocuments::Default);
    assert_eq!(
        sends[0].optional_params.get("return_documents"),
        Some(&Json::Bool(true))
    );
    assert_eq!(sends[0].timeout_seconds, None);
    assert!(!sends[0].is_async);
}

#[test]
fn zero_valued_options_reach_the_mapping() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.top_n = Some(0);
    call.rank_fields = Some(Vec::new());
    call.max_chunks_per_doc = Some(0);
    call.max_tokens_per_doc = Some(0);
    call.kwargs = with(&[("instruction", Json::Text(String::from("brief")))]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("zero options dispatch");
    let options = ops.options();
    assert_eq!(options[0].top_n, Some(0));
    assert_eq!(options[0].rank_fields, Some(Vec::new()));
    assert_eq!(options[0].max_chunks_per_doc, Some(0));
    assert_eq!(options[0].max_tokens_per_doc, Some(0));
    assert_eq!(options[0].instruction.as_deref(), Some("brief"));
    let sends = ops.sends();
    assert_eq!(sends[0].top_n, Some(0));
    assert_eq!(sends[0].rank_fields, Some(Vec::new()));
}

#[test]
fn a_string_timeout_becomes_a_float_after_mapping() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[("timeout", Json::Text(String::from("2.5")))]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the string timeout dispatches");
    assert_eq!(ops.sends()[0].timeout_seconds, Some(2.5_f64));
}

#[test]
fn a_bad_timeout_fails_before_the_provider_call() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[("timeout", Json::Text(String::from("bad")))]);
    let error = dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect_err("a bad timeout raises");
    assert_eq!(error.message, "could not convert string to float: 'bad'");
    assert!(matches!(error.class, ErrorClass::ApiConnection(_)));
    assert_eq!(ops.options().len(), 1);
    assert!(ops.sends().is_empty());
}

#[test]
fn only_the_boolean_true_arerank_selects_async() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[("arerank", Json::Bool(true))]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the async call dispatches");
    assert!(ops.sends()[0].is_async);
    assert!(!ops.sends()[0].generic_params.contains_key("arerank"));

    let mut call = call("cohere");
    call.kwargs = with(&[("arerank", Json::Number(Number::from(1_i64)))]);
    let mut logging = logging_state();
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the numeric arerank dispatches");
    assert!(!ops.sends()[1].is_async);
}

#[test]
fn mock_response_does_not_bypass_the_provider_call() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[("mock_response", Json::Text(String::from("hello")))]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the call still dispatches");
    assert_eq!(ops.sends().len(), 1);
    assert!(
        ops.options()[0]
            .non_default_keys
            .contains(&String::from("mock_response"))
    );
}

#[test]
fn drop_params_uses_the_call_value_then_the_global_value() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut global = defaults();
    global.drop_params = true;
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[("drop_params", Json::Bool(false))]);
    dispatch(
        &identity,
        &ops,
        &global,
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the call dispatches");
    assert!(ops.options()[0].drop_params);

    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[("drop_params", Json::Bool(true))]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the call dispatches");
    assert!(ops.options()[1].drop_params);

    let mut logging = logging_state();
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call("cohere"),
        &mut logging,
    )
    .expect("the call dispatches");
    assert!(!ops.options()[2].drop_params);
}

#[test]
fn logger_params_carry_the_call_identity_and_no_invented_stream_response() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[
        ("litellm_call_id", Json::Text(String::from("lc-1"))),
        ("proxy_server_request", Json::Object(IndexMap::new())),
        ("model_info", Json::Object(IndexMap::new())),
    ]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the call dispatches");
    let sends = ops.sends();
    assert_eq!(
        sends[0].litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("lc-1")))
    );
    assert_eq!(
        sends[0].litellm_params.get("preset_cache_key"),
        Some(&Json::Null)
    );
    assert_eq!(
        sends[0].litellm_params.get("stream_response"),
        Some(&Json::Object(IndexMap::new()))
    );
    assert_eq!(
        logging.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("lc-1")))
    );
    assert_eq!(
        logging.litellm_params.get("stream_response"),
        Some(&Json::Object(IndexMap::new()))
    );

    let mut call = call("cohere");
    call.kwargs = with(&[
        ("preset_cache_key", Json::Text(String::from("preset-1"))),
        (
            "stream_response",
            Json::Object(IndexMap::from([(String::from("kept"), Json::Bool(true))])),
        ),
    ]);
    let mut logging = logging_state();
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the call dispatches");
    let sends = ops.sends();
    assert_eq!(
        sends[1].litellm_params.get("preset_cache_key"),
        Some(&Json::Text(String::from("preset-1")))
    );
    assert_eq!(
        logging.litellm_params.get("preset_cache_key"),
        Some(&Json::Text(String::from("preset-1")))
    );
}

#[test]
fn version_params_choose_the_cohere_client() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.max_chunks_per_doc = Some(0);
    call.kwargs = with(&[("api_base", Json::Text(String::from("https://d/v1/rerank")))]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the v1 call dispatches");
    assert_eq!(ops.options()[0].config, SelectedConfig::Cohere);

    let mut logging = logging_state();
    let mut call = call("cohere");
    call.max_tokens_per_doc = Some(0);
    call.kwargs = with(&[("api_base", Json::Text(String::from("https://d")))]);

    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the v2 call dispatches");
    assert_eq!(ops.options()[1].config, SelectedConfig::CohereV2);
}

#[test]
fn a_mapping_failure_raises_through_the_error_mapping() {
    let ops = FakeOps {
        options: Mutex::new(Vec::new()),
        sends: Mutex::new(Vec::new()),
        mapping_failure: Some(String::from(
            "Invalid parameter: stream. Only ['query', 'documents'] are supported.",
        )),
        send_failure: None,
    };
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let error = dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call("cohere"),
        &mut logging,
    )
    .expect_err("the mapping raises");
    assert!(error.message.starts_with("Invalid parameter: stream"));
    assert!(ops.sends().is_empty());
}

#[test]
fn return_documents_null_is_absent_and_false_is_sent() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.return_documents = ReturnDocuments::Null;
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the null return_documents dispatches");
    assert_eq!(ops.sends()[0].optional_params.get("return_documents"), None);

    let mut logging = logging_state();
    let mut call = call("cohere");
    call.return_documents = ReturnDocuments::Value(false);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the false return_documents dispatches");
    assert_eq!(
        ops.sends()[1].optional_params.get("return_documents"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn the_logger_gets_the_call_user() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://d/v1"),
    );
    let mut logging = logging_state();
    let mut call = call("cohere");
    call.kwargs = with(&[("user", Json::Text(String::from("u-1")))]);
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call,
        &mut logging,
    )
    .expect("the call dispatches");
    assert_eq!(logging.user.as_deref(), Some("u-1"));
}

#[rstest]
#[case::azure_ai("azure_ai", "rerank-v1", SelectedConfig::AzureAi)]
#[case::hosted_vllm("hosted_vllm", "rerank-v1", SelectedConfig::HostedVllm)]
#[case::huggingface("huggingface", "rerank-v1", SelectedConfig::HuggingFace)]
#[case::deepinfra("deepinfra", "rerank-v1", SelectedConfig::Deepinfra)]
#[case::vertex_ai("vertex_ai", "rerank-v1", SelectedConfig::VertexAi)]
#[case::fireworks_ai("fireworks_ai", "rerank-v1", SelectedConfig::FireworksAi)]
#[case::watsonx("watsonx", "rerank-v1", SelectedConfig::Watsonx)]
#[case::dashscope("dashscope", "rerank-v1", SelectedConfig::Dashscope)]
#[case::qwencloud("qwencloud", "rerank-v1", SelectedConfig::Qwencloud)]
#[case::qwen_ai_platform("qwen_ai_platform", "rerank-v1", SelectedConfig::QwenAiPlatform)]
#[case::nvidia_retrieval("nvidia_nim", "rerank-v1", SelectedConfig::NvidiaRetrieval)]
#[case::nvidia_ranking(
    "nvidia_nim",
    "ranking/nvidia/rerank-qa",
    SelectedConfig::NvidiaRanking
)]
#[case::unknown_falls_back("openai", "rerank-v1", SelectedConfig::Cohere)]
fn every_provider_slug_selects_its_config(
    #[case] slug: &str,
    #[case] model: &str,
    #[case] expected: SelectedConfig,
) {
    let identity = ProviderIdentity::Custom(slug.to_owned());
    assert_eq!(
        super::selection::selected_config(&identity, model, None, &[]),
        expected
    );
}

#[test]
fn the_catalog_identity_resolves_a_declared_provider() {
    let catalog = BundledCatalog::default().get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let empty_environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known,
        custom_providers: &custom,
        environment: &empty_environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let identity = CatalogIdentity { context: &context };
    let resolved = identity
        .resolve(&ResolutionRequest {
            model: "rerank-v1",
            custom_llm_provider: Some("cohere"),
            api_base: None,
            api_key: None,
        })
        .expect("a declared provider resolves");
    assert_eq!(resolved.provider.slug(), "cohere");
    assert_eq!(resolved.model, "rerank-v1");
}

#[test]
fn watsonx_reads_its_environment_credentials() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("watsonx".to_owned()),
        None,
        None,
    );
    let mut logging = logging_state();
    dispatch(
        &identity,
        &ops,
        &defaults(),
        &environment(&[
            ("WATSONX_API_KEY", "env-key"),
            ("WATSONX_API_BASE", "https://env.watsonx.test"),
        ]),
        call("watsonx"),
        &mut logging,
    )
    .expect("watsonx takes the environment credentials");
    let sends = ops.sends();
    assert_eq!(sends[0].config, SelectedConfig::Watsonx);
    assert_eq!(sends[0].api_key.as_deref(), Some("env-key"));
    assert_eq!(
        sends[0].api_base.as_deref(),
        Some("https://env.watsonx.test")
    );
    assert_eq!(sends[0].optional_params.get("token"), None);
}

#[test]
fn watsonx_without_credentials_raises_the_key_error() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("watsonx".to_owned()),
        None,
        None,
    );
    let mut logging = logging_state();
    let error = dispatch(
        &identity,
        &ops,
        &defaults(),
        &BTreeMap::new(),
        call("watsonx"),
        &mut logging,
    )
    .expect_err("watsonx needs a key");
    assert_eq!(
        error.message,
        "Error: Watsonx API key not set. Set WATSONX_API_KEY in environment variables or pass in as parameter - 'api_key='."
    );
    assert!(ops.sends().is_empty());
}
