#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_anthropic::skills::wire::{DeleteSkillResponse, ListSkillsResponse, Skill};
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
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::api::skills::fixtures::{
    API_BASE, CALL_ID, anthropic_settings, redactor, sample_deleted, sample_skill,
};
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::get_delete::handler::SkillsHandlerError;
use crate::api::skills::get_delete::hosted::{
    HostedSkillRequest, HostedSkillsHandler, HostedSkillsHandlers,
};
use crate::api::skills::list::call::{ListSkillsCall, SkillListing};
use crate::api::skills::list::handler::{ListSkillsHttpRequest, SkillsListHttpHandler};
use crate::api::skills::list::hosted::{HostedListSkillsRequest, HostedSkillsLister};
use crate::api::skills::list::service::ListSkillsService;

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

fn core_settings() -> CoreSettings {
    CoreSettings {
        request_timeout_seconds: 6000.0_f64,
        request_timeout_explicit: false,
        litellm_request_timeout: None,
        model_timeout_overrides: Vec::new(),
    }
}

pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "list_skills".to_owned(),
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

pub(crate) fn params() -> SkillsCallParams {
    SkillsCallParams::default()
}

pub(crate) fn hosted_params() -> SkillsCallParams {
    let mut params = params();
    params.custom_llm_provider = Some("litellm_proxy".to_owned());
    params
}

pub(crate) fn params_with_base(api_base: Option<&str>) -> SkillsCallParams {
    let mut params = params();
    params.api_base = api_base.map(str::to_owned);
    params
}

pub(crate) fn params_with_query(extra_query: IndexMap<String, Json>) -> SkillsCallParams {
    let mut params = params();
    params.extra_query = Some(extra_query);
    params
}

pub(crate) fn params_with_timeout(timeout: Option<Duration>) -> SkillsCallParams {
    let mut params = params();
    params.timeout = timeout;
    params
}

pub(crate) fn params_for(provider: &str) -> SkillsCallParams {
    let mut params = params();
    params.custom_llm_provider = Some(provider.to_owned());
    params
}

pub(crate) fn call(values: &IndexMap<String, Json>) -> ListSkillsCall<'_> {
    call_with(params(), None, None, None, values)
}

pub(crate) fn call_with<'call>(
    params: SkillsCallParams,
    limit: Option<i64>,
    page: Option<&'call str>,
    source: Option<&'call str>,
    kwargs: &'call IndexMap<String, Json>,
) -> ListSkillsCall<'call> {
    ListSkillsCall {
        params,
        limit,
        page,
        source,
        kwargs,
        litellm_call_id: Some(CALL_ID),
    }
}

pub(crate) fn listing<'call>(
    call: &'call ListSkillsCall<'call>,
    logging: &'call mut LoggingState,
) -> SkillListing<'call> {
    SkillListing { call, logging }
}

pub(crate) fn query(values: &[(&str, &str)]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn map(values: &[(&str, Json)]) -> IndexMap<String, Json> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn sample_list_response() -> ListSkillsResponse {
    ListSkillsResponse {
        data: vec![sample_skill()],
        next_page: Some("page-2".to_owned()),
        has_more: true,
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

pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) query_params: Vec<(String, String)>,
    pub(crate) headers: HeaderMap,
    pub(crate) timeout: Duration,
    pub(crate) provider: String,
}

fn recorded(request: &ListSkillsHttpRequest<'_>) -> Recorded {
    Recorded {
        url: request.url.clone(),
        query_params: request.query_params.clone(),
        headers: request.headers.clone(),
        timeout: request.timeout,
        provider: request.custom_llm_provider.to_owned(),
    }
}

pub(crate) struct FakeHttp {
    pub(crate) requests: Mutex<Vec<Recorded>>,
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
            requests: Mutex::new(Vec::new()),
            status,
        }
    }
}

impl SkillsListHttpHandler for FakeHttp {
    fn list_skills(
        &self,
        request: ListSkillsHttpRequest<'_>,
    ) -> impl Future<Output = Result<ListSkillsResponse, SkillsHandlerError>> + Send {
        let mut requests = match self.requests.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        requests.push(recorded(&request));
        drop(requests);
        ready(self.status.map_or_else(
            || Ok(sample_list_response()),
            |status| Err(handler_error(status)),
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HostedCall {
    pub(crate) limit: i64,
    pub(crate) offset: i64,
    pub(crate) principal: Option<Json>,
    pub(crate) litellm_call_id: Option<String>,
}

fn hosted_call(request: &HostedListSkillsRequest<'_>) -> HostedCall {
    HostedCall {
        limit: request.limit,
        offset: request.offset,
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
        ready(Ok(sample_deleted()))
    }
}

impl HostedSkillsLister for FakeHosted {
    fn list_skills(
        &self,
        request: HostedListSkillsRequest<'_>,
    ) -> impl Future<Output = Result<ListSkillsResponse, SkillsHandlerError>> + Send {
        self.record(&request);
        ready(Ok(sample_list_response()))
    }
}

impl FakeHosted {
    fn record(&self, request: &HostedListSkillsRequest<'_>) {
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

    pub(crate) fn service(&self) -> ListSkillsService<'_, FakeHttp, FakeHosted> {
        ListSkillsService {
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

pub(crate) fn unauthenticated_settings() -> AnthropicSettings {
    AnthropicSettings {
        api_key: None,
        auth_token: None,
        api_base: API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}
