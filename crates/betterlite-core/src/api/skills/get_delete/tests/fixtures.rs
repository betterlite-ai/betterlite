#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_anthropic::skills::wire::{DeleteSkillResponse, Skill};
use betterlite_context::core_settings::CoreSettings;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::skills::get_delete::call::{SkillCall, SkillRequest, SkillsCallParams};
use crate::api::skills::get_delete::error::SkillsCallError;
use crate::api::skills::get_delete::handler::{
    SkillHttpRequest, SkillsHandlerError, SkillsHttpHandler,
};
use crate::api::skills::get_delete::hosted::{
    HostedSkillRequest, HostedSkillsHandler, HostedSkillsHandlers,
};
use crate::api::skills::get_delete::service::SkillsService;

pub(crate) const SKILL_ID: &str = "skill_1";
pub(crate) const API_BASE: &str = "https://api.anthropic.com";
pub(crate) const CALL_ID: &str = "call-1";

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "get_skill".to_owned(),
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

pub(crate) fn anthropic_settings() -> AnthropicSettings {
    AnthropicSettings {
        api_key: Some(SecretString::from("sk-ant-test")),
        auth_token: None,
        api_base: API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

fn core_settings() -> CoreSettings {
    CoreSettings {
        request_timeout_seconds: 6000.0_f64,
        request_timeout_explicit: false,
        litellm_request_timeout: None,
        model_timeout_overrides: Vec::new(),
    }
}

pub(crate) fn sample_skill() -> Skill {
    Skill {
        id: SKILL_ID.to_owned(),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        display_title: Some("Fixture".to_owned()),
        latest_version: Some("1".to_owned()),
        source: "custom".to_owned(),
        r#type: "skill".to_owned(),
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
        description: None,
        search_score: None,
    }
}

pub(crate) fn sample_deleted() -> DeleteSkillResponse {
    DeleteSkillResponse {
        id: SKILL_ID.to_owned(),
        r#type: "skill_deleted".to_owned(),
    }
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn object(pairs: &[(&str, Json)]) -> Json {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    Json::Object(values)
}

pub(crate) fn metadata_with(principal: Json) -> Json {
    object(&[("user_api_key_auth", principal)])
}

pub(crate) fn kwargs() -> IndexMap<String, Json> {
    IndexMap::new()
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

pub(crate) fn call<'call>(
    skill_id: &'call str,
    kwargs: &'call IndexMap<String, Json>,
) -> SkillCall<'call> {
    call_with(params(), skill_id, kwargs)
}

pub(crate) fn skill_request<'call>(
    call: &'call SkillCall<'call>,
    logging: &'call mut LoggingState,
) -> SkillRequest<'call> {
    SkillRequest { call, logging }
}

pub(crate) fn call_with<'call>(
    params: SkillsCallParams,
    skill_id: &'call str,
    kwargs: &'call IndexMap<String, Json>,
) -> SkillCall<'call> {
    SkillCall {
        skill_id,
        params,
        kwargs,
        litellm_call_id: Some(CALL_ID),
    }
}

pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: HeaderMap,
    pub(crate) timeout: Duration,
    pub(crate) provider: String,
}

fn recorded(request: &SkillHttpRequest<'_>) -> Recorded {
    Recorded {
        url: request.url.clone(),
        headers: request.headers.clone(),
        timeout: request.timeout,
        provider: request.custom_llm_provider.to_owned(),
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

pub(crate) fn call_error(status_code: u16) -> SkillsCallError {
    SkillsCallError::Handler(handler_error(status_code))
}

pub(crate) struct FakeHttp {
    pub(crate) get_requests: Mutex<Vec<Recorded>>,
    pub(crate) delete_requests: Mutex<Vec<Recorded>>,
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
            get_requests: Mutex::new(Vec::new()),
            delete_requests: Mutex::new(Vec::new()),
            status,
        }
    }

    fn answer<T>(
        &self,
        request: &SkillHttpRequest<'_>,
        requests: &Mutex<Vec<Recorded>>,
        success: fn() -> T,
    ) -> Result<T, SkillsHandlerError> {
        let mut calls = match requests.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        calls.push(recorded(request));
        drop(calls);
        self.status
            .map_or_else(|| Ok(success()), |status| Err(handler_error(status)))
    }
}

impl SkillsHttpHandler for FakeHttp {
    fn get_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        ready(self.answer(&request, &self.get_requests, sample_skill))
    }

    fn delete_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send {
        ready(self.answer(&request, &self.delete_requests, sample_deleted))
    }
}

#[derive(Clone)]
pub(crate) struct HostedCall {
    pub(crate) skill_id: String,
    pub(crate) principal: Option<Json>,
    pub(crate) litellm_call_id: Option<String>,
}

fn hosted_call(request: &HostedSkillRequest<'_>) -> HostedCall {
    HostedCall {
        skill_id: request.skill_id.to_owned(),
        principal: request.user_api_key_auth.cloned(),
        litellm_call_id: request.litellm_call_id.map(str::to_owned),
    }
}

pub(crate) struct FakeHosted {
    calls: Arc<Mutex<Vec<HostedCall>>>,
    deletes: Arc<Mutex<Vec<HostedCall>>>,
}

fn hosted_record<T>(
    calls: &Mutex<Vec<HostedCall>>,
    request: &HostedSkillRequest<'_>,
    success: fn() -> T,
) -> T {
    calls
        .lock()
        .expect("the fake records its calls")
        .push(hosted_call(request));
    success()
}

impl HostedSkillsHandler for FakeHosted {
    fn get_skill(
        &self,
        request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        ready(Ok(hosted_record(&self.calls, &request, sample_skill)))
    }

    fn delete_skill(
        &self,
        request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send {
        ready(Ok(hosted_record(&self.deletes, &request, sample_deleted)))
    }
}

pub(crate) struct HostedFixture {
    handlers: HostedSkillsHandlers<FakeHosted>,
    created: Arc<Mutex<usize>>,
    pub(crate) calls: Arc<Mutex<Vec<HostedCall>>>,
    pub(crate) deletes: Arc<Mutex<Vec<HostedCall>>>,
}

fn hosted_fixture() -> HostedFixture {
    let created = Arc::new(Mutex::new(0_usize));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let deletes = Arc::new(Mutex::new(Vec::new()));
    let builds = Arc::clone(&created);
    let get_calls = Arc::clone(&calls);
    let delete_calls = Arc::clone(&deletes);
    HostedFixture {
        handlers: HostedSkillsHandlers {
            create: Box::new(move || {
                let mut count = builds.lock().expect("the factory counts");
                *count = count.checked_add(1).expect("the build count fits");
                drop(count);
                Arc::new(FakeHosted {
                    calls: Arc::clone(&get_calls),
                    deletes: Arc::clone(&delete_calls),
                })
            }),
            cached: Mutex::new(None),
        },
        created,
        calls,
        deletes,
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

    pub(crate) fn service(&self) -> SkillsService<'_, FakeHttp, FakeHosted> {
        SkillsService {
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

pub(crate) fn header_values(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn extra_headers(name: &'static str, value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    );
    headers
}
