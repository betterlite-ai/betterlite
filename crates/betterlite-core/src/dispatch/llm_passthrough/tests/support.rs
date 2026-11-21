use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use bytes::Bytes;

use crate::api::passthrough_call::{
    PassthroughBody, PassthroughCallArgs, PassthroughCallError, PassthroughReply,
    PassthroughRouterCalls,
};
use crate::api::video_router_calls::route::VideoRouteView;
use crate::dispatch::llm_passthrough::route::{PassthroughRouteInput, RouteFlag};
use crate::dispatch::llm_passthrough::{BlockedModels, PassthroughCallPort};
use crate::dispatch::tests::support::Trace;
use crate::operation::llm_passthrough::LlmPassthroughRequest;

pub(super) struct Ports {
    pub(super) router: Option<Arc<dyn PassthroughCallPort>>,
    pub(super) view: Option<Arc<dyn VideoRouteView + Send + Sync>>,
    pub(super) blocked: Arc<dyn BlockedModels>,
    pub(super) direct: Arc<dyn PassthroughCallPort>,
}

pub(super) fn call_args(model: &str) -> PassthroughCallArgs {
    let method = String::from("POST");
    let endpoint = String::from("/v1/messages");
    let extra = Object::new();
    PassthroughCallArgs {
        method,
        endpoint,
        model: model.to_owned(),
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
        request_query_params: None,
        request_headers: None,
        content: None,
        data: None,
        files: None,
        json: None,
        params: None,
        cookies: None,
        client: None,
        extra,
    }
}

pub(super) fn route_input(model: &str) -> PassthroughRouteInput {
    PassthroughRouteInput {
        model: model.to_owned(),
        team_id: None,
        is_proxy_admin_without_team: RouteFlag(false),
        has_api_key_or_base: RouteFlag(false),
        has_user_config: RouteFlag(false),
        router_settings_override: None,
        user_model: None,
    }
}

pub(super) fn override_settings() -> Object {
    let mut settings = Object::new();
    settings.insert(String::from("fallbacks"), JsonValue::Array(Vec::new()));
    settings.insert(
        String::from("context_window_fallbacks"),
        JsonValue::Array(Vec::new()),
    );
    settings.insert(
        String::from("content_policy_fallbacks"),
        JsonValue::Array(Vec::new()),
    );
    settings.insert(String::from("num_retries"), JsonValue::from(3_u32));
    settings.insert(String::from("timeout"), JsonValue::from(30_u32));
    settings.insert(
        String::from("model_group_retry_policy"),
        JsonValue::Object(Object::new()),
    );
    settings.insert(
        String::from("routing_strategy"),
        JsonValue::String(String::from("usage-based-routing")),
    );
    settings.insert(String::from("enable_tag_filtering"), JsonValue::Bool(true));
    settings
}

pub(super) fn reply() -> PassthroughReply {
    PassthroughReply {
        status: 200_u16,
        headers: Vec::new(),
        body: PassthroughBody::Full(Box::new(Bytes::from_static(b"{}"))),
    }
}

pub(super) const fn failure(status: u16) -> PassthroughCallError {
    PassthroughCallError::UpstreamStatus {
        status,
        headers: Vec::new(),
        body: Bytes::from_static(b"upstream failed"),
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Outcome {
    Reply,
    Fail(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RecordedCall {
    pub(super) model: String,
    pub(super) extra: Object,
}

#[derive(Clone, Default)]
pub(super) struct Calls {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl Calls {
    pub(super) fn push(&self, args: &PassthroughCallArgs) {
        let model = args.model.clone();
        let extra = args.extra.clone();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall { model, extra });
    }

    pub(super) fn recorded(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
pub(super) struct FakeView {
    pub(super) recognized: RouteFlag,
    pub(super) team_map: Vec<(&'static str, &'static str, &'static str)>,
    pub(super) team_public: Vec<&'static str>,
    pub(super) model_names: Vec<&'static str>,
    pub(super) pass_through_all: RouteFlag,
    pub(super) default_or_pattern: RouteFlag,
    pub(super) deployment_names: Vec<&'static str>,
}

impl VideoRouteView for FakeView {
    fn is_recognized_model(&self, _model: &str) -> bool {
        self.recognized.0
    }

    fn map_team_model(&self, model: &str, team_id: &str) -> Option<String> {
        self.team_map
            .iter()
            .find(|(name, team, _mapped)| *name == model && *team == team_id)
            .map(|(_name, _team, mapped)| String::from(*mapped))
    }

    fn team_public_model_names_has(&self, model: &str) -> bool {
        self.team_public.iter().any(|name| *name == model)
    }

    fn model_names_has(&self, model: &str) -> bool {
        self.model_names.iter().any(|name| *name == model)
    }

    fn pass_through_all_models(&self) -> bool {
        self.pass_through_all.0
    }

    fn has_default_or_pattern(&self) -> bool {
        self.default_or_pattern.0
    }

    fn deployment_names_has(&self, model: &str) -> bool {
        self.deployment_names.iter().any(|name| *name == model)
    }
}

pub(super) const fn empty_view() -> FakeView {
    FakeView {
        recognized: RouteFlag(false),
        team_map: Vec::new(),
        team_public: Vec::new(),
        model_names: Vec::new(),
        pass_through_all: RouteFlag(false),
        default_or_pattern: RouteFlag(false),
        deployment_names: Vec::new(),
    }
}

pub(super) struct RecordingBlocked {
    pub(super) blocked: Vec<&'static str>,
    pub(super) trace: Trace,
}

impl BlockedModels for RecordingBlocked {
    fn all_blocked(&self, model: &str, team_id: Option<&str>) -> bool {
        self.trace
            .push(&format!("blocked:{model}:{}", team_id.unwrap_or("")));
        self.blocked.iter().any(|name| *name == model)
    }
}

pub(super) struct RecordingCallee {
    pub(super) label: &'static str,
    pub(super) trace: Trace,
    pub(super) outcome: Outcome,
    pub(super) calls: Calls,
}

#[async_trait]
impl PassthroughCallPort for RecordingCallee {
    async fn call(
        &self,
        args: PassthroughCallArgs,
    ) -> Result<PassthroughReply, PassthroughCallError> {
        self.trace.push(self.label);
        self.calls.push(&args);
        match self.outcome {
            Outcome::Reply => Ok(reply()),
            Outcome::Fail(status) => Err(failure(status)),
        }
    }
}

pub(super) struct SdkRouter {
    pub(super) trace: Trace,
}

impl PassthroughRouterCalls for SdkRouter {
    fn allm_passthrough_route(
        &self,
        _args: PassthroughCallArgs,
    ) -> impl Future<Output = Result<PassthroughReply, PassthroughCallError>> + Send {
        self.trace.push("sdk");
        ready(Ok(reply()))
    }
}

pub(super) fn sdk_router(trace: &Trace) -> Arc<dyn PassthroughCallPort> {
    Arc::new(SdkRouter {
        trace: trace.clone(),
    })
}

pub(super) fn ports_with(
    view: Option<FakeView>,
    blocked: Vec<&'static str>,
    calls: &Calls,
    trace: &Trace,
) -> Ports {
    let router: Arc<dyn PassthroughCallPort> = Arc::new(RecordingCallee {
        label: "router",
        trace: trace.clone(),
        outcome: Outcome::Reply,
        calls: calls.clone(),
    });
    let direct: Arc<dyn PassthroughCallPort> = Arc::new(RecordingCallee {
        label: "direct",
        trace: trace.clone(),
        outcome: Outcome::Reply,
        calls: calls.clone(),
    });
    let blocked_port: Arc<dyn BlockedModels> = Arc::new(RecordingBlocked {
        blocked,
        trace: trace.clone(),
    });
    Ports {
        router: Some(router),
        view: view.map(view_of),
        blocked: blocked_port,
        direct,
    }
}

pub(super) fn request(
    args: PassthroughCallArgs,
    route: PassthroughRouteInput,
    ports: &Ports,
) -> LlmPassthroughRequest {
    LlmPassthroughRequest {
        args,
        route,
        router: ports.router.clone(),
        view: ports.view.clone(),
        blocked: Arc::clone(&ports.blocked),
        direct: Arc::clone(&ports.direct),
    }
}

pub(super) fn failing_router_request(
    status: u16,
    trace: &Trace,
    calls: &Calls,
) -> LlmPassthroughRequest {
    let router: Arc<dyn PassthroughCallPort> = Arc::new(RecordingCallee {
        label: "router",
        trace: trace.clone(),
        outcome: Outcome::Fail(status),
        calls: calls.clone(),
    });
    let view: Arc<dyn VideoRouteView + Send + Sync> = Arc::new(FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    });
    let blocked: Arc<dyn BlockedModels> = Arc::new(RecordingBlocked {
        blocked: Vec::new(),
        trace: trace.clone(),
    });
    let direct: Arc<dyn PassthroughCallPort> = Arc::new(RecordingCallee {
        label: "direct",
        trace: trace.clone(),
        outcome: Outcome::Reply,
        calls: calls.clone(),
    });
    LlmPassthroughRequest {
        args: call_args("gpt-upstream"),
        route: route_input("gpt-upstream"),
        router: Some(router),
        view: Some(view),
        blocked,
        direct,
    }
}

fn view_of(view: FakeView) -> Arc<dyn VideoRouteView + Send + Sync> {
    Arc::new(view)
}
