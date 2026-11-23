use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_anthropic::skills::config::BaseSkillsApiConfig;
use betterlite_anthropic::skills::error::SkillsError;
use betterlite_anthropic::skills::wire::{DeleteSkillResponse, ListSkillsResponse, Skill};
use betterlite_core::api::skills::get_delete::handler::{
    SkillHttpRequest, SkillsHandlerError, SkillsHttpHandler,
};
use betterlite_core::api::skills::list::handler::{ListSkillsHttpRequest, SkillsListHttpHandler};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::watsonx_audio::error::body_text;
use http::Method;
use http::header::HeaderMap;
use url::Url;

pub(super) mod create;
mod multipart;
mod tests;

// LiteLLM's get and list go through the cached httpx client, whose 600 second total applies whatever REQUEST_TIMEOUT says
const GET_SKILLS_TIMEOUT: Duration = Duration::from_secs(600);

// The verb one skill call sends: read is what both get and list use.
#[derive(Clone, Copy)]
enum Action {
    Read,
    Post,
    Delete,
}

impl Action {
    const fn method(self) -> Method {
        match self {
            Self::Read => Method::GET,
            Self::Post => Method::POST,
            Self::Delete => Method::DELETE,
        }
    }
}

// The deadline LiteLLM gives one call and the status its timeouts take: the httpx handler answers APITimeoutError.
#[derive(Clone, Copy)]
struct Exchange {
    action: Action,
    after: Duration,
    timeout_status: Option<u16>,
}

impl Exchange {
    const READ: Self = Self {
        action: Action::Read,
        after: GET_SKILLS_TIMEOUT,
        timeout_status: None,
    };

    const fn delete(timeout: Duration) -> Self {
        Self {
            action: Action::Delete,
            after: timeout,
            timeout_status: Some(408_u16),
        }
    }

    const fn create(timeout: Duration) -> Self {
        Self {
            action: Action::Post,
            after: timeout,
            timeout_status: Some(408_u16),
        }
    }
}

// One upstream call: the parts each operation assembles, the config that reads its answer, and its deadline.
struct Call<'request> {
    url: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    body: RequestBody,
    config: &'request dyn BaseSkillsApiConfig,
    exchange: Exchange,
}

impl<'request> Call<'request> {
    const fn new(
        url: String,
        headers: HeaderMap,
        query: Vec<(String, String)>,
        config: &'request dyn BaseSkillsApiConfig,
        exchange: Exchange,
    ) -> Self {
        Self {
            url,
            headers,
            query,
            body: RequestBody::Empty,
            config,
            exchange,
        }
    }
}

// A get or delete call: the skill request's parts under the verb its exchange names, with no body.
impl<'request> From<(Exchange, SkillHttpRequest<'request>)> for Call<'request> {
    fn from((exchange, request): (Exchange, SkillHttpRequest<'request>)) -> Self {
        Self::new(
            request.url,
            request.headers,
            Vec::new(),
            request.config,
            exchange,
        )
    }
}

const fn list_call(request: ListSkillsHttpRequest<'_>) -> Call<'_> {
    let config = request.config;
    let query = request.query_params;
    Call::new(request.url, request.headers, query, config, Exchange::READ)
}

// The transform each operation applies to the provider's answer body.
type Transform<T> = fn(&dyn BaseSkillsApiConfig, &[u8]) -> Result<T, SkillsError>;
const GET_SKILL: Transform<Skill> = |config, body| config.transform_get_skill_response(body);
const DELETE_SKILL: Transform<DeleteSkillResponse> =
    |config, body| config.transform_delete_skill_response(body);
const LIST_SKILLS: Transform<ListSkillsResponse> =
    |config, body| config.transform_list_skills_response(body);

// The session's http side: the calls every skill operation can make without a database.
pub(super) struct HttpSkills {
    pub(super) client: ProxiedClient,
}

impl SkillsHttpHandler for HttpSkills {
    fn get_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        let call = Call::from((Exchange::READ, request));
        answered(&self.client, call, GET_SKILL)
    }

    fn delete_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send {
        let call = Call::from((Exchange::delete(request.timeout), request));
        answered(&self.client, call, DELETE_SKILL)
    }
}

impl SkillsListHttpHandler for HttpSkills {
    fn list_skills(
        &self,
        request: ListSkillsHttpRequest<'_>,
    ) -> impl Future<Output = Result<ListSkillsResponse, SkillsHandlerError>> + Send {
        let call = list_call(request);
        answered(&self.client, call, LIST_SKILLS)
    }
}

// What every operation does with the transport's answer: refuse a non-2xx status, then read the body as the provider writes it.
async fn answered<T>(
    client: &ProxiedClient,
    call: Call<'_>,
    transform: Transform<T>,
) -> Result<T, SkillsHandlerError> {
    let config = call.config;
    let response = exchange(client, call).await?;
    ensure_success(&response, config)?;
    transform(config, &response.body).map_err(SkillsHandlerError::from)
}

async fn exchange(
    client: &ProxiedClient,
    call: Call<'_>,
) -> Result<UpstreamResponse, SkillsHandlerError> {
    let url = Url::parse(&call.url).map_err(|_invalid| {
        provider_failure(
            500,
            format!("the skills url {} is not a URL", call.url),
            true,
        )
    })?;
    let exchange = call.exchange;
    let request = OutboundRequest {
        method: exchange.action.method(),
        url,
        query: call.query,
        headers: call.headers,
        body: call.body,
        deadline: exchange.after,
    };
    client
        .send_buffered(request)
        .await
        .map_err(|error| transport_failure(&error, exchange))
}

fn ensure_success(
    response: &UpstreamResponse,
    config: &dyn BaseSkillsApiConfig,
) -> Result<(), SkillsHandlerError> {
    if (200_u16..300_u16).contains(&response.status) {
        return Ok(());
    }
    let class = config.get_error_class(
        &body_text(&response.body),
        response.status,
        &BTreeMap::new(),
    );
    Err(SkillsHandlerError::from(class))
}

fn transport_failure(error: &TransportError, deadline: Exchange) -> SkillsHandlerError {
    let timed_out = matches!(
        error,
        TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
    );
    match deadline.timeout_status {
        Some(status) if timed_out => provider_failure(
            status,
            format!(
                "Connection timed out after {} seconds.",
                deadline.after.as_secs_f64()
            ),
            false,
        ),
        Some(_status) | None => provider_failure(500, error.to_string(), true),
    }
}

const fn provider_failure(
    status_code: u16,
    message: String,
    synthesized: bool,
) -> SkillsHandlerError {
    SkillsHandlerError::Provider {
        status_code,
        message,
        status_code_is_synthesized: synthesized,
        body_code: None,
        body_message: None,
    }
}
