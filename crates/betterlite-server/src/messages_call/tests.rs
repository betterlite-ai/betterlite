#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::mock_testing_params::Settings as MockTestingParams;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;

use super::outcome::{MessagesDispatch, MessagesOutcome, MessagesRouting};
use super::plan::{
    MessagesRoute, MessagesRouteInput, RouteFlag, plan_messages_route, team_id_from_data,
};
use super::steps::{
    CallStepError, GATED_MOCK_PARAM_NAMES, mock_params_disallowed, required_param_missing,
};
use super::{CallFailure, Context, MessagesCallError, MessagesCallParts, MessagesCallSteps};
use crate::keepalive_interval::DeploymentLookup;
use crate::route_read_through::{ModelNotFoundCause as _, ModelReadThrough};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Flag(pub bool);

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn object(entries: &[(&str, JsonValue)]) -> JsonMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[derive(Default)]
struct FakeDispatch {
    calls: Mutex<Vec<JsonMap>>,
}

#[async_trait]
impl MessagesDispatch for FakeDispatch {
    async fn dispatch(&self, data: JsonMap) -> Result<MessagesOutcome, MessagesCallError> {
        self.calls.lock().unwrap().push(data.clone());
        Ok(MessagesOutcome::Message {
            body: data,
            hidden: JsonMap::new(),
        })
    }
}

struct FakeRouting {
    blocked: Flag,
    blocked_queries: Mutex<Vec<(String, Option<String>)>>,
    team_model: Option<String>,
    team_queries: Mutex<Vec<(Option<String>, String)>>,
    recognized: BTreeSet<String>,
    granted: Mutex<Option<String>>,
    model_names: BTreeSet<String>,
    team_public: BTreeSet<String>,
    pass_through: Flag,
    default_or_pattern: Flag,
    deployment_names: BTreeSet<String>,
    a2a: Flag,
    router_calls: Mutex<Vec<(JsonMap, bool)>>,
    a2a_calls: Mutex<Vec<JsonMap>>,
}

const fn empty_routing() -> FakeRouting {
    FakeRouting {
        blocked: Flag(false),
        blocked_queries: Mutex::new(Vec::new()),
        team_model: None,
        team_queries: Mutex::new(Vec::new()),
        recognized: BTreeSet::new(),
        granted: Mutex::new(None),
        model_names: BTreeSet::new(),
        team_public: BTreeSet::new(),
        pass_through: Flag(false),
        default_or_pattern: Flag(false),
        deployment_names: BTreeSet::new(),
        a2a: Flag(false),
        router_calls: Mutex::new(Vec::new()),
        a2a_calls: Mutex::new(Vec::new()),
    }
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn routing_with_groups(values: &[&str]) -> FakeRouting {
    let mut routing = empty_routing();
    routing.model_names = names(values);
    routing.recognized = names(values);
    routing
}

impl DeploymentLookup for FakeRouting {
    fn deployments_for(&self, _model_name: &str) -> Vec<JsonValue> {
        Vec::new()
    }
}

#[async_trait]
impl MessagesRouting for FakeRouting {
    fn all_blocked(&self, model: &str, team_id: Option<&str>) -> bool {
        self.blocked_queries
            .lock()
            .unwrap()
            .push((model.to_owned(), team_id.map(str::to_owned)));
        self.blocked.0
    }

    fn map_team_model(&self, model: Option<&str>, team_id: &str) -> Option<String> {
        self.team_queries
            .lock()
            .unwrap()
            .push((model.map(str::to_owned), team_id.to_owned()));
        self.team_model.clone()
    }

    fn is_recognized_model(&self, model: &str) -> bool {
        self.recognized.contains(model) || self.granted.lock().unwrap().as_deref() == Some(model)
    }

    fn model_names_has(&self, model: &str) -> bool {
        self.model_names.contains(model)
    }

    fn team_public_model_names_has(&self, model: &str) -> bool {
        self.team_public.contains(model)
    }

    fn pass_through_all_models(&self) -> bool {
        self.pass_through.0
    }

    fn has_default_or_pattern(&self) -> bool {
        self.default_or_pattern.0
    }

    fn is_deployment_name(&self, model: &str) -> bool {
        self.deployment_names.contains(model)
    }

    async fn anthropic_messages(
        &self,
        data: JsonMap,
        specific_deployment: bool,
    ) -> Result<MessagesOutcome, MessagesCallError> {
        self.router_calls
            .lock()
            .unwrap()
            .push((data.clone(), specific_deployment));
        Ok(MessagesOutcome::Message {
            body: data,
            hidden: JsonMap::new(),
        })
    }

    async fn a2a_agent(&self, data: JsonMap) -> Option<Result<MessagesOutcome, MessagesCallError>> {
        self.a2a_calls.lock().unwrap().push(data.clone());
        if !self.a2a.0 {
            return None;
        }
        Some(Ok(MessagesOutcome::Message {
            body: data,
            hidden: JsonMap::new(),
        }))
    }
}

struct FakeReadThrough {
    answers: bool,
    calls: Mutex<Vec<String>>,
    grants: Option<Arc<FakeRouting>>,
}

#[async_trait]
impl ModelReadThrough for FakeReadThrough {
    async fn attempt_model(&self, model: &str) -> bool {
        self.calls.lock().unwrap().push(model.to_owned());
        if let Some(routing) = &self.grants {
            *routing.granted.lock().unwrap() = Some(model.to_owned());
        }
        self.answers
    }
}

fn routing_port(port: Arc<FakeRouting>) -> Arc<dyn MessagesRouting> {
    port
}

fn read_through_port(port: Arc<FakeReadThrough>) -> Arc<dyn ModelReadThrough> {
    port
}

fn dispatch_port(port: Arc<FakeDispatch>) -> Arc<dyn MessagesDispatch> {
    port
}

fn call_steps(
    dispatch: Arc<FakeDispatch>,
    routing: Option<Arc<FakeRouting>>,
    read_through: Option<Arc<FakeReadThrough>>,
    allowed: bool,
    user_model: Option<&str>,
) -> MessagesCallSteps {
    MessagesCallSteps::new(
        "anthropic_messages",
        MessagesCallParts {
            dispatch: dispatch_port(dispatch),
            routing: routing.map(routing_port),
            read_through: read_through.map(read_through_port),
            mock: MockTestingParams { allowed },
            user_model: user_model.map(str::to_owned),
        },
    )
}

#[tokio::test]
async fn steps_name_their_route_type_in_not_found() {
    let steps = MessagesCallSteps::new(
        "agenerate_content",
        MessagesCallParts {
            dispatch: dispatch_port(Arc::new(FakeDispatch::default())),
            routing: None,
            read_through: None,
            mock: MockTestingParams { allowed: false },
            user_model: None,
        },
    );
    let Err(error) = steps
        .call(object(&[("model", text("mystery"))]), false)
        .await
    else {
        panic!("an unserved model is refused");
    };
    assert_eq!(error.status(), 400);
    assert_eq!(
        error.http_detail(),
        Some(JsonValue::Object(object(&[(
            "error",
            text(
                "agenerate_content: Invalid model name passed in model=mystery. Call `/v1/models` to view available models for your key."
            )
        )])))
    );
}

fn input(model: Option<&str>, has_model: bool) -> MessagesRouteInput {
    MessagesRouteInput {
        model: model.map(str::to_owned),
        has_model,
        team_id: None,
        is_proxy_admin: false,
        has_api_key_or_base: RouteFlag(false),
        has_user_config: RouteFlag(false),
        has_override: RouteFlag(false),
        user_model: None,
    }
}

const fn router_plan(specific_deployment: bool) -> MessagesRoute {
    MessagesRoute::Router {
        specific_deployment,
    }
}

fn assert_missing(data: &JsonMap, route: &str, endpoint: &str, param: &str) {
    let error = required_param_missing(route, data).expect_err("the parameter is missing");
    assert_eq!(
        error.to_string(),
        format!("{endpoint}: Missing required parameter: '{param}'.")
    );
    assert_eq!(error.error_type(), Some("invalid_request_error"));
    assert_eq!(error.param(), Some(param));
    assert_eq!(error.code(), Some("400"));
    let CallStepError::MissingParam {
        route: named,
        param: missing,
    } = error
    else {
        panic!("a missing parameter is not a mock gate failure");
    };
    assert_eq!(named, endpoint);
    assert_eq!(missing, param);
}

#[rstest]
#[case::completion("acompletion", "/chat/completions", "messages")]
#[case::embedding("aembedding", "/embeddings", "input")]
#[case::responses("aresponses", "/responses", "input")]
fn one_required_param_of_the_route_is_missing(
    #[case] route: &str,
    #[case] endpoint: &str,
    #[case] param: &str,
) {
    assert_missing(&JsonMap::new(), route, endpoint, param);
}

#[test]
fn the_batch_parameters_fail_in_litellm_order() {
    let mut data = JsonMap::new();
    assert_missing(&data, "acreate_batch", "/batches", "input_file_id");
    data.insert("input_file_id".to_owned(), text("file"));
    assert_missing(&data, "acreate_batch", "/batches", "endpoint");
    data.insert("endpoint".to_owned(), text("/v1/chat/completions"));
    assert_missing(&data, "acreate_batch", "/batches", "completion_window");
    data.insert("completion_window".to_owned(), text("24h"));
    required_param_missing("acreate_batch", &data).unwrap();
}

#[test]
fn a_null_parameter_counts_as_missing() {
    let data = object(&[("messages", JsonValue::Null)]);
    assert_missing(&data, "acompletion", "/chat/completions", "messages");
}

#[rstest]
#[case::messages("anthropic_messages")]
#[case::unlisted("allm_passthrough_route")]
fn a_route_without_the_table_never_fails_the_step(#[case] route: &str) {
    required_param_missing(route, &JsonMap::new()).unwrap();
    required_param_missing(route, &object(&[("messages", JsonValue::Null)])).unwrap();
}

#[rstest]
#[case::fallbacks("mock_testing_fallbacks")]
#[case::context_fallbacks("mock_testing_context_fallbacks")]
#[case::content_policy("mock_testing_content_policy_fallbacks")]
#[case::rate_limit("mock_testing_rate_limit_error")]
#[case::timeout("mock_timeout")]
#[case::delay("mock_delay")]
fn a_gated_mock_param_is_refused_when_null(#[case] name: &str) {
    let error = mock_params_disallowed(&object(&[(name, JsonValue::Null)]), false)
        .expect_err("the gated name is refused");
    assert_eq!(
        error,
        CallStepError::MockParamsDisabled {
            names: vec![name.to_owned()]
        }
    );
}

#[test]
fn the_mock_names_come_back_in_constant_order_with_litellms_detail() {
    assert_eq!(
        GATED_MOCK_PARAM_NAMES,
        [
            "mock_testing_fallbacks",
            "mock_testing_context_fallbacks",
            "mock_testing_content_policy_fallbacks",
            "mock_testing_rate_limit_error",
            "mock_timeout",
            "mock_delay",
        ]
    );
    let mut data = JsonMap::new();
    data.insert("mock_delay".to_owned(), text("1"));
    data.insert("mock_testing_fallbacks".to_owned(), JsonValue::Bool(false));
    let error = mock_params_disallowed(&data, false).expect_err("both names are gated");
    assert_eq!(
        error.to_string(),
        "Mock testing request params are disabled on this proxy: mock_testing_fallbacks, mock_delay. An admin can enable them by setting `general_settings.dangerously_allow_mock_testing_request_params: true` in config.yaml. This setting cannot be changed from the Admin UI or the API."
    );
    let CallStepError::MockParamsDisabled { names } = error else {
        panic!("a gated name is not a missing parameter");
    };
    assert_eq!(
        names,
        vec!["mock_testing_fallbacks".to_owned(), "mock_delay".to_owned()]
    );
}

#[test]
fn an_allowed_proxy_takes_every_gated_name() {
    let entries: Vec<(&str, JsonValue)> = GATED_MOCK_PARAM_NAMES
        .into_iter()
        .map(|name| (name, JsonValue::Null))
        .collect();
    mock_params_disallowed(&object(&entries), true).unwrap();
}

#[test]
fn a_name_outside_the_gated_list_passes() {
    mock_params_disallowed(&object(&[("mock_response", text("x"))]), false).unwrap();
}

#[tokio::test]
async fn tag_filtering_is_gone_from_the_data_the_callee_gets() {
    let dispatch = Arc::new(FakeDispatch::default());
    let steps = call_steps(Arc::clone(&dispatch), None, None, false, Some("m"));
    let data = object(&[
        ("model", text("m")),
        ("enable_tag_filtering", JsonValue::Bool(true)),
    ]);
    steps.call(data, false).await.unwrap();
    let sent = dispatch.calls.lock().unwrap()[0].clone();
    assert!(!sent.contains_key("enable_tag_filtering"));
}

#[test]
fn an_api_key_runs_direct_without_a_router_and_through_it_with_one() {
    let routing = empty_routing();
    let mut input = input(Some("m"), true);
    input.has_api_key_or_base = RouteFlag(true);
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        router_plan(false)
    );
    assert_eq!(plan_messages_route(&input, None), MessagesRoute::Direct);
}

#[test]
fn the_api_key_branch_comes_before_user_config_and_the_override() {
    let routing = empty_routing();
    let mut input = input(Some("m"), true);
    input.has_api_key_or_base = RouteFlag(true);
    input.has_user_config = RouteFlag(true);
    input.has_override = RouteFlag(true);
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        router_plan(false)
    );
}

#[test]
fn user_config_answers_user_config_before_the_override() {
    let routing = empty_routing();
    let mut input = input(Some("m"), true);
    input.has_user_config = RouteFlag(true);
    input.has_override = RouteFlag(true);
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        MessagesRoute::UserConfig
    );
}

#[test]
fn the_override_branch_runs_before_the_blocked_check() {
    let mut routing = empty_routing();
    routing.blocked = Flag(true);
    let mut input = input(Some("m"), true);
    input.has_override = RouteFlag(true);
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        router_plan(false)
    );
    assert!(routing.blocked_queries.lock().unwrap().is_empty());
}

#[test]
fn a_fully_blocked_model_is_blocked_with_its_team() {
    let mut routing = empty_routing();
    routing.blocked = Flag(true);
    let mut input = input(Some("m"), true);
    input.team_id = Some("team".to_owned());
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        MessagesRoute::Blocked
    );
    let blocked_queries = routing.blocked_queries.lock().unwrap();
    let expected = [("m".to_owned(), Some("team".to_owned()))];
    assert_eq!(blocked_queries.as_slice(), expected.as_slice());
}

#[test]
fn the_blocked_check_skips_an_empty_and_a_non_string_model() {
    let mut routing = empty_routing();
    routing.blocked = Flag(true);
    assert_eq!(
        plan_messages_route(&input(Some(""), true), Some(&routing)),
        MessagesRoute::NotFound
    );
    assert_eq!(
        plan_messages_route(&input(None, true), Some(&routing)),
        MessagesRoute::NotFound
    );
    assert_eq!(routing.blocked_queries.lock().unwrap().len(), 0);
}

#[test]
fn a_body_without_a_model_key_is_missing_model() {
    let routing = routing_with_groups(&["m"]);
    assert_eq!(
        plan_messages_route(&input(None, false), Some(&routing)),
        MessagesRoute::MissingModel
    );
}

#[test]
fn a_recognized_model_routes_to_the_router() {
    let routing = routing_with_groups(&["m"]);
    assert_eq!(
        plan_messages_route(&input(Some("m"), true), Some(&routing)),
        router_plan(false)
    );
}

#[test]
fn an_admin_without_a_team_may_call_a_team_public_model() {
    let mut routing = empty_routing();
    routing.team_public = names(&["shared"]);
    let mut input = input(Some("shared"), true);
    input.is_proxy_admin = true;
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        router_plan(false)
    );
    input.is_proxy_admin = false;
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        MessagesRoute::NotFound
    );
}

#[test]
fn a_team_public_model_needs_the_admin_and_no_team() {
    let mut routing = empty_routing();
    routing.team_public = names(&["shared"]);
    let mut input = input(Some("shared"), true);
    input.is_proxy_admin = true;
    input.team_id = Some("team".to_owned());
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        MessagesRoute::NotFound
    );
}

#[test]
fn pass_through_answers_direct_before_the_default_pattern_and_deployment_names() {
    let mut routing = empty_routing();
    routing.pass_through = Flag(true);
    routing.default_or_pattern = Flag(true);
    routing.deployment_names = names(&["openai/m"]);
    assert_eq!(
        plan_messages_route(&input(Some("openai/m"), true), Some(&routing)),
        MessagesRoute::Direct
    );
}

#[test]
fn a_default_or_pattern_answers_the_router_before_a_deployment_name() {
    let mut routing = empty_routing();
    routing.default_or_pattern = Flag(true);
    routing.deployment_names = names(&["openai/m"]);
    assert_eq!(
        plan_messages_route(&input(Some("openai/m"), true), Some(&routing)),
        router_plan(false)
    );
}

#[test]
fn a_deployment_name_answers_a_specific_deployment() {
    let mut routing = empty_routing();
    routing.deployment_names = names(&["openai/m"]);
    assert_eq!(
        plan_messages_route(&input(Some("openai/m"), true), Some(&routing)),
        router_plan(true)
    );
}

#[test]
fn an_a2a_model_answers_the_a2a_agent() {
    let routing = empty_routing();
    assert_eq!(
        plan_messages_route(&input(Some("a2a/agent"), true), Some(&routing)),
        MessagesRoute::A2aAgent
    );
}

#[test]
fn an_unlisted_model_is_not_found() {
    let routing = empty_routing();
    assert_eq!(
        plan_messages_route(&input(Some("mystery"), true), Some(&routing)),
        MessagesRoute::NotFound
    );
}

#[test]
fn a_model_in_the_names_that_is_not_recognized_is_not_found() {
    let mut routing = empty_routing();
    routing.model_names = names(&["m"]);
    assert_eq!(
        plan_messages_route(&input(Some("m"), true), Some(&routing)),
        MessagesRoute::NotFound
    );
}

#[test]
fn a_non_string_model_is_absent_from_every_list() {
    let mut routing = empty_routing();
    routing.recognized = names(&["m"]);
    routing.model_names = names(&["m"]);
    routing.deployment_names = names(&["m"]);
    assert_eq!(
        plan_messages_route(&input(None, true), Some(&routing)),
        MessagesRoute::NotFound
    );
    routing.pass_through = Flag(true);
    assert_eq!(
        plan_messages_route(&input(None, true), Some(&routing)),
        MessagesRoute::Direct
    );
}

#[test]
fn without_a_router_a_user_model_runs_direct_and_nothing_is_not_found() {
    let routing = routing_with_groups(&["m"]);
    let mut input = input(Some("m"), true);
    input.user_model = Some("m".to_owned());
    assert_eq!(plan_messages_route(&input, None), MessagesRoute::Direct);
    input.user_model = None;
    assert_eq!(plan_messages_route(&input, None), MessagesRoute::NotFound);
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        router_plan(false)
    );
}

#[test]
fn the_team_id_comes_from_metadata_first_then_litellm_metadata() {
    let both = object(&[
        (
            "metadata",
            JsonValue::Object(object(&[("user_api_key_team_id", text("meta"))])),
        ),
        (
            "litellm_metadata",
            JsonValue::Object(object(&[("user_api_key_team_id", text("litellm"))])),
        ),
    ]);
    assert_eq!(team_id_from_data(&both).as_deref(), Some("meta"));
    let fallback = object(&[
        ("metadata", JsonValue::Object(JsonMap::new())),
        (
            "litellm_metadata",
            JsonValue::Object(object(&[("user_api_key_team_id", text("litellm"))])),
        ),
    ]);
    assert_eq!(team_id_from_data(&fallback).as_deref(), Some("litellm"));
    let wrong_type = object(&[(
        "metadata",
        JsonValue::Object(object(&[("user_api_key_team_id", JsonValue::from(7_i64))])),
    )]);
    assert_eq!(team_id_from_data(&wrong_type), None);
    let not_an_object = object(&[("metadata", text("metadata"))]);
    assert_eq!(team_id_from_data(&not_an_object), None);
}

#[tokio::test]
async fn call_reaches_the_dispatch_for_the_direct_plan() {
    let dispatch = Arc::new(FakeDispatch::default());
    let steps = call_steps(Arc::clone(&dispatch), None, None, false, Some("cli-model"));
    let outcome = steps
        .call(object(&[("model", text("m"))]), false)
        .await
        .unwrap();
    assert!(matches!(outcome, MessagesOutcome::Message { .. }));
    assert_eq!(dispatch.calls.lock().unwrap().len(), 1);
}

#[rstest]
#[case::model_group("m", false)]
#[case::deployment_name("openai/m", true)]
#[tokio::test]
async fn call_reaches_the_router_for_the_router_plan(
    #[case] model: &str,
    #[case] specific_deployment: bool,
) {
    let mut fake = routing_with_groups(&["m"]);
    fake.deployment_names = names(&["openai/m"]);
    let routing = Arc::new(fake);
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&routing)),
        None,
        false,
        None,
    );
    steps
        .call(object(&[("model", text(model))]), false)
        .await
        .unwrap();
    let calls = routing.router_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, specific_deployment);
}

#[tokio::test]
async fn call_reaches_the_a2a_port_and_not_found_when_it_answers_none() {
    let mut fake = empty_routing();
    fake.a2a = Flag(true);
    let answering = Arc::new(fake);
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&answering)),
        None,
        false,
        None,
    );
    steps
        .call(object(&[("model", text("a2a/agent"))]), false)
        .await
        .unwrap();
    assert_eq!(answering.a2a_calls.lock().unwrap().len(), 1);
    let silent = Arc::new(empty_routing());
    let silent_steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&silent)),
        None,
        false,
        None,
    );
    let Err(error) = silent_steps
        .call(object(&[("model", text("a2a/agent"))]), false)
        .await
    else {
        panic!("an a2a agent that answers none is not found");
    };
    assert_eq!(error.status(), 400);
    assert_eq!(silent.a2a_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_blocked_call_answers_the_permission_denied_text() {
    let mut routing = empty_routing();
    routing.blocked = Flag(true);
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::new(routing)),
        None,
        false,
        None,
    );
    let Err(error) = steps.call(object(&[("model", text("m"))]), false).await else {
        panic!("a blocked model is refused");
    };
    assert_eq!(error.status(), 403);
    assert_eq!(
        error.to_string(),
        "litellm.PermissionDeniedError: Model is blocked"
    );
    assert_eq!(error.http_detail(), None);
    assert!(error.model_not_found().is_none());
}

#[tokio::test]
async fn a_missing_model_answers_a_bad_request_naming_the_field() {
    let routing = Arc::new(empty_routing());
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(routing),
        None,
        false,
        None,
    );
    let Err(error) = steps
        .call(object(&[("max_tokens", JsonValue::from(10_i64))]), false)
        .await
    else {
        panic!("a body without a model is refused");
    };
    assert_eq!(error.status(), 400);
    assert_eq!(error.to_string(), "model is a required field");
    assert_eq!(error.http_detail(), None);
}

#[tokio::test]
async fn a_user_config_body_answers_the_deviation() {
    let routing = Arc::new(empty_routing());
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(routing),
        None,
        false,
        None,
    );
    let Err(error) = steps
        .call(
            object(&[
                ("model", text("m")),
                ("user_config", JsonValue::Object(JsonMap::new())),
            ]),
            false,
        )
        .await
    else {
        panic!("user_config is refused");
    };
    assert_eq!(error.status(), 400);
    assert_eq!(
        error.to_string(),
        "user_config is not supported on /v1/messages"
    );
}

#[tokio::test]
async fn an_unserved_model_answers_the_litellm_not_found() {
    let routing = Arc::new(empty_routing());
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(routing),
        None,
        false,
        None,
    );
    let Err(error) = steps
        .call(object(&[("model", text("mystery"))]), false)
        .await
    else {
        panic!("an unserved model is refused");
    };
    assert_eq!(error.status(), 400);
    assert_eq!(
        error.to_string(),
        "anthropic_messages: Invalid model name passed in model=mystery. Call `/v1/models` to view available models for your key."
    );
    assert_eq!(
        error.http_detail(),
        Some(JsonValue::Object(object(&[(
            "error",
            text(
                "anthropic_messages: Invalid model name passed in model=mystery. Call `/v1/models` to view available models for your key."
            )
        )])))
    );
    let not_found = error.model_not_found().unwrap();
    assert!(not_found.retryable_with_model_read_through);
    assert_eq!(
        not_found.spend_log_error_message,
        "anthropic_messages: Invalid model name passed in. Call `/v1/models` to view available models for your key."
    );
}

#[tokio::test]
async fn the_not_found_of_an_empty_model_names_the_empty_string() {
    let steps = call_steps(Arc::new(FakeDispatch::default()), None, None, false, None);
    let Err(error) = steps.call(object(&[("model", text(""))]), false).await else {
        panic!("an empty model is refused");
    };
    assert_eq!(error.status(), 400);
    assert_eq!(
        error.to_string(),
        "anthropic_messages: Invalid model name passed in model=. Call `/v1/models` to view available models for your key."
    );
}

#[tokio::test]
async fn a_read_through_that_answers_true_runs_the_attempt_again() {
    let routing = Arc::new(empty_routing());
    let read_through = Arc::new(FakeReadThrough {
        answers: true,
        calls: Mutex::new(Vec::new()),
        grants: Some(Arc::clone(&routing)),
    });
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&routing)),
        Some(Arc::clone(&read_through)),
        false,
        None,
    );
    steps
        .call(object(&[("model", text("m"))]), false)
        .await
        .unwrap();
    assert_eq!(
        read_through.calls.lock().unwrap().as_slice(),
        ["m".to_owned()].as_slice()
    );
    assert_eq!(routing.blocked_queries.lock().unwrap().len(), 2);
    assert_eq!(routing.router_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_read_through_that_answers_false_keeps_the_first_error() {
    let routing = Arc::new(empty_routing());
    let read_through = Arc::new(FakeReadThrough {
        answers: false,
        calls: Mutex::new(Vec::new()),
        grants: Some(Arc::clone(&routing)),
    });
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&routing)),
        Some(Arc::clone(&read_through)),
        false,
        None,
    );
    let Err(error) = steps.call(object(&[("model", text("m"))]), false).await else {
        panic!("the read-through did not grant the model");
    };
    assert_eq!(error.status(), 400);
    assert_eq!(read_through.calls.lock().unwrap().len(), 1);
    assert_eq!(routing.blocked_queries.lock().unwrap().len(), 1);
}

#[test]
fn a_call_failure_keeps_its_status_and_a_context_keeps_its_own() {
    let failure = CallFailure {
        status: Some(429),
        message: "slow down".to_owned(),
        error_type: Some("rate_limit_error".to_owned()),
        param: None,
        headers: BTreeMap::new(),
        timeout: None,
        cooldown: None,
    };
    let error = MessagesCallError::Call(failure);
    assert_eq!(error.status(), 429);
    assert_eq!(error.to_string(), "slow down");
    assert_eq!(error.http_detail(), None);
    let stateless = CallFailure {
        status: None,
        message: "no port".to_owned(),
        error_type: None,
        param: None,
        headers: BTreeMap::new(),
        timeout: None,
        cooldown: None,
    };
    assert_eq!(MessagesCallError::Call(stateless).status(), 500);
    let context = Context {
        status: 503,
        message: "polyfill failed".to_owned(),
    };
    let context_error = MessagesCallError::Context(context);
    assert_eq!(context_error.status(), 503);
    assert_eq!(context_error.to_string(), "polyfill failed");
}

#[test]
fn the_step_errors_answer_their_status_and_detail() {
    let missing = MessagesCallError::Steps(CallStepError::MissingParam {
        route: "/chat/completions".to_owned(),
        param: "messages".to_owned(),
    });
    assert_eq!(missing.status(), 400);
    assert_eq!(missing.http_detail(), None);
    let mock = MessagesCallError::Steps(CallStepError::MockParamsDisabled {
        names: vec!["mock_delay".to_owned()],
    });
    assert_eq!(mock.status(), 400);
    assert_eq!(
        mock.http_detail(),
        Some(JsonValue::Object(object(&[(
            "error",
            text(
                "Mock testing request params are disabled on this proxy: mock_delay. An admin can enable them by setting `general_settings.dangerously_allow_mock_testing_request_params: true` in config.yaml. This setting cannot be changed from the Admin UI or the API."
            )
        )])))
    );
}

mod edges;
mod overrides;
mod standalone;
