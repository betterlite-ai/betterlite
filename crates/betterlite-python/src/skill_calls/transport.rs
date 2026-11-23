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

// One upstream call: what the operation assembles, the config that reads its answer, its verb and its deadline.
struct Call<'request> {
    method: Method,
    url: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    body: RequestBody,
    config: &'request dyn BaseSkillsApiConfig,
    after: Duration,
    timeout_status: Option<u16>,
}

impl<'request> Call<'request> {
    // A get or delete call: the skill request's parts under the verb and deadline given, with no body.
    const fn new(
        request: SkillHttpRequest<'request>,
        method: Method,
        after: Duration,
        timeout_status: Option<u16>,
    ) -> Self {
        Self {
            method,
            url: request.url,
            headers: request.headers,
            query: Vec::new(),
            body: RequestBody::Empty,
            config: request.config,
            after,
            timeout_status,
        }
    }
}

// A list call: the list request's own query under the cached client's 600 second total, with no body.
const fn list_call(request: ListSkillsHttpRequest<'_>) -> Call<'_> {
    Call {
        method: Method::GET,
        url: request.url,
        headers: request.headers,
        query: request.query_params,
        body: RequestBody::Empty,
        config: request.config,
        after: GET_SKILLS_TIMEOUT,
        timeout_status: None,
    }
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
        let call = Call::new(request, Method::GET, GET_SKILLS_TIMEOUT, None);
        answered(&self.client, call, GET_SKILL)
    }

    fn delete_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send {
        let after = request.timeout;
        let call = Call::new(request, Method::DELETE, after, Some(408_u16));
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
        let message = format!("the skills url {} is not a URL", call.url);
        provider_failure(500, message, true)
    })?;
    let after = call.after;
    let timeout_status = call.timeout_status;
    let request = OutboundRequest {
        method: call.method,
        url,
        query: call.query,
        headers: call.headers,
        body: call.body,
        deadline: after,
    };
    client
        .send_buffered(request)
        .await
        .map_err(|error| transport_failure(&error, after, timeout_status))
}

fn ensure_success(
    response: &UpstreamResponse,
    config: &dyn BaseSkillsApiConfig,
) -> Result<(), SkillsHandlerError> {
    if (200_u16..300_u16).contains(&response.status) {
        return Ok(());
    }
    let body = body_text(&response.body);
    let class = config.get_error_class(&body, response.status, &BTreeMap::new());
    Err(SkillsHandlerError::from(class))
}

// The status LiteLLM gives one call's timeout: the httpx handler answers APITimeoutError, everything else is a server error.
fn transport_failure(
    error: &TransportError,
    after: Duration,
    timeout_status: Option<u16>,
) -> SkillsHandlerError {
    let timed_out = matches!(
        error,
        TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
    );
    match timeout_status {
        Some(status) if timed_out => {
            let message = format!(
                "Connection timed out after {} seconds.",
                after.as_secs_f64()
            );
            provider_failure(status, message, false)
        }
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
