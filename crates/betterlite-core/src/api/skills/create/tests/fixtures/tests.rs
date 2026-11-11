#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_anthropic::skills::wire::{CreateSkillRequest, DeleteSkillResponse, Skill};
use betterlite_context::core_settings::CoreSettings;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;

use crate::api::skills::create::call::{CreateSkillCall, SkillCreation};
use crate::api::skills::create::handler::{CreateSkillHttpRequest, SkillsCreateHttpHandler};
use crate::api::skills::create::hosted::{HostedCreateSkillRequest, HostedSkillsCreator};
use crate::api::skills::create::service::CreateSkillService;
use crate::api::skills::fixtures::{API_BASE, CALL_ID, anthropic_settings, redactor, sample_skill};
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::get_delete::handler::SkillsHandlerError;
use crate::api::skills::get_delete::hosted::{
    HostedSkillRequest, HostedSkillsHandler, HostedSkillsHandlers,
};

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn map(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "create_skill".to_owned(),
            start_time: moment(),
            litellm_call_id: CALL_ID.to_owned(),
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
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

fn core_settings() -> CoreSettings {
    CoreSettings {
        request_timeout_seconds: 6000.0_f64,
        request_timeout_explicit: false,
        litellm_request_timeout: None,
        model_timeout_overrides: Vec::new(),
    }
}

fn params() -> SkillsCallParams {
    SkillsCallParams::default()
}

pub(crate) fn hosted_params() -> SkillsCallParams {
    SkillsCallParams {
        api_key: None,
        api_base: None,
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: Some("litellm_proxy".to_owned()),
    }
}

pub(crate) fn params_with_base(api_base: Option<&str>) -> SkillsCallParams {
    SkillsCallParams {
        api_key: None,
        api_base: api_base.map(str::to_owned),
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    }
}

pub(crate) fn call(kwargs: &IndexMap<String, Json>) -> CreateSkillCall<'_> {
    call_with(params(), None, None, None, kwargs)
}

pub(crate) fn call_with<'call>(
    params: SkillsCallParams,
    display_title: Option<&'call str>,
    files: Option<&'call [Json]>,
    extra_body: Option<&'call IndexMap<String, Json>>,
    kwargs: &'call IndexMap<String, Json>,
) -> CreateSkillCall<'call> {
    CreateSkillCall {
        params,
        display_title,
        files,
        extra_body,
        kwargs,
        litellm_call_id: Some(CALL_ID),
    }
}

pub(crate) fn skill_creation<'call>(
    call: &'call CreateSkillCall<'call>,
    logging: &'call mut LoggingState,
) -> SkillCreation<'call> {
    SkillCreation { call, logging }
}

pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: HeaderMap,
    pub(crate) timeout: Duration,
    pub(crate) provider: String,
    pub(crate) body: CreateSkillRequest,
}

fn recorded(request: &CreateSkillHttpRequest<'_>) -> Recorded {
    Recorded {
        url: request.url.clone(),
        headers: request.headers.clone(),
        timeout: request.timeout,
        provider: request.custom_llm_provider.to_owned(),
        body: request.body.clone(),
    }
}

fn handler_error(status_code: u16) -> SkillsHandlerError {
    SkillsHandlerError::Provider {
        status_code,
        message: "the provider refused".to_owned(),
        status_code_is_synthesized: false,
        body_code: None,
        body_message: None,
    }
}

pub(crate) struct FakeHttp {
    pub(crate) creates: Mutex<Vec<Recorded>>,
    status: Option<u16>,
}

impl FakeHttp {
    fn new() -> Self {
        Self::with_status(None)
    }

    fn failing(status: u16) -> Self {
        Self::with_status(Some(status))
    }

    fn with_status(status: Option<u16>) -> Self {
        Self {
            creates: Mutex::new(Vec::new()),
            status,
        }
    }

    fn answer(&self) -> Result<Skill, SkillsHandlerError> {
        self.status
            .map_or_else(|| Ok(sample_skill()), |status| Err(handler_error(status)))
    }
}

impl SkillsCreateHttpHandler for FakeHttp {
    fn create_skill(
        &self,
        request: CreateSkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        let mut creates = match self.creates.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        creates.push(recorded(&request));
        drop(creates);
        ready(self.answer())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HostedCall {
    pub(crate) display_title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) instructions: Option<String>,
    pub(crate) files: Option<Vec<Json>>,
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    pub(crate) user_id: Option<String>,
    pub(crate) principal: Option<Json>,
    pub(crate) litellm_call_id: Option<String>,
}

fn hosted_call(request: &HostedCreateSkillRequest<'_>) -> HostedCall {
    HostedCall {
        display_title: request.display_title.map(str::to_owned),
        description: request.description.map(str::to_owned),
        instructions: request.instructions.map(str::to_owned),
        files: request.files.map(<[Json]>::to_vec),
        metadata: request.metadata.cloned(),
        user_id: request.user_id.map(str::to_owned),
        principal: request.user_api_key_auth.cloned(),
        litellm_call_id: request.litellm_call_id.map(str::to_owned),
    }
}

pub(crate) struct FakeHosted {
    calls: Arc<Mutex<Vec<HostedCall>>>,
}

impl HostedSkillsHandler for FakeHosted {
    fn get_skill(
        &self,
        _request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        ready(Ok(sample_skill()))
    }

    fn delete_skill(
        &self,
        _request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send {
        ready(Ok(DeleteSkillResponse {
            id: "skill_1".to_owned(),
            r#type: "skill_deleted".to_owned(),
        }))
    }
}

impl HostedSkillsCreator for FakeHosted {
    fn create_skill(
        &self,
        request: HostedCreateSkillRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        self.record(&request);
        ready(Ok(sample_skill()))
    }
}

impl FakeHosted {
    fn record(&self, request: &HostedCreateSkillRequest<'_>) {
        self.calls
            .lock()
            .expect("the fake records its calls")
            .push(hosted_call(request));
    }
}

pub(crate) struct HostedFixture {
    handlers: HostedSkillsHandlers<FakeHosted>,
    created: Arc<Mutex<usize>>,
    pub(crate) calls: Arc<Mutex<Vec<HostedCall>>>,
}

fn hosted_fixture() -> HostedFixture {
    let created = Arc::new(Mutex::new(0_usize));
    let calls: Arc<Mutex<Vec<HostedCall>>> = Arc::new(Mutex::new(Vec::new()));
    let builds = Arc::clone(&created);
    let recorded = Arc::clone(&calls);
    HostedFixture {
        handlers: HostedSkillsHandlers {
            create: Box::new(move || {
                let mut count = builds.lock().expect("the factory counts");
                *count = count.checked_add(1).expect("the build count fits");
                drop(count);
                Arc::new(FakeHosted {
                    calls: Arc::clone(&recorded),
                })
            }),
            cached: Mutex::new(None),
        },
        created,
        calls,
    }
}

pub(crate) struct Fixture {
    pub(crate) http: FakeHttp,
    pub(crate) hosted: HostedFixture,
    pub(crate) anthropic: AnthropicSettings,
    pub(crate) core: CoreSettings,
    redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self::with_http(FakeHttp::new())
    }

    pub(crate) fn failing(status: u16) -> Self {
        Self::with_http(FakeHttp::failing(status))
    }

    fn with_http(http: FakeHttp) -> Self {
        Self {
            http,
            hosted: hosted_fixture(),
            anthropic: anthropic_settings(),
            core: core_settings(),
            redactor: redactor(),
        }
    }

    pub(crate) fn service(&self) -> CreateSkillService<'_, FakeHttp, FakeHosted> {
        CreateSkillService {
            http: &self.http,
            hosted: &self.hosted.handlers,
            anthropic: &self.anthropic,
            core: &self.core,
            redactor: &self.redactor,
        }
    }

    pub(crate) fn builds(&self) -> usize {
        *self.hosted.created.lock().expect("the factory counts")
    }
}

pub(crate) fn unauthenticated_settings() -> AnthropicSettings {
    AnthropicSettings {
        api_key: None,
        auth_token: None,
        api_base: API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}
