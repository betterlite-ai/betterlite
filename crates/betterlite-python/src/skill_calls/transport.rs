use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_anthropic::skills::config::BaseSkillsApiConfig;
use betterlite_anthropic::skills::wire::{
    CreateSkillRequest, DeleteSkillResponse, ListSkillsResponse, Skill,
};
use betterlite_core::api::skills::create::handler::{
    CreateSkillHttpRequest, SkillsCreateHttpHandler,
};
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
use http::header::{CONTENT_TYPE, HeaderMap};
use url::Url;
use uuid::Uuid;

use crate::skill_calls::convert;

mod multipart;
mod tests;

// LiteLLM's get and list go through the cached httpx client, whose 600 second total applies whatever REQUEST_TIMEOUT says
const GET_SKILLS_TIMEOUT: Duration = Duration::from_secs(600);

// The read deadline LiteLLM gives one call, and the status its timeouts take: the httpx handler answers APITimeoutError.
#[derive(Clone, Copy)]
struct Exchange {
    after: Duration,
    timeout_status: Option<u16>,
}

// One upstream call: the parts each operation assembles, with the deadline its LiteLLM handler gives it.
struct UpstreamCall {
    method: Method,
    url: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    body: RequestBody,
    deadline: Exchange,
}

// The session's http side: the calls every skill operation can make without a database.
pub(super) struct HttpSkills {
    pub(super) client: ProxiedClient,
}

impl SkillsHttpHandler for HttpSkills {
    fn get_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        async move {
            let response = exchange(
                &self.client,
                UpstreamCall {
                    method: Method::GET,
                    url: request.url,
                    headers: request.headers,
                    query: Vec::new(),
                    body: RequestBody::Empty,
                    deadline: Exchange {
                        after: GET_SKILLS_TIMEOUT,
                        timeout_status: None,
                    },
                },
            )
            .await?;
            ensure_success(&response, request.config)?;
            request
                .config
                .transform_get_skill_response(&response.body)
                .map_err(SkillsHandlerError::from)
        }
    }

    fn delete_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send {
        async move {
            let response = exchange(
                &self.client,
                UpstreamCall {
                    method: Method::DELETE,
                    url: request.url,
                    headers: request.headers,
                    query: Vec::new(),
                    body: RequestBody::Empty,
                    deadline: Exchange {
                        after: request.timeout,
                        timeout_status: Some(408_u16),
                    },
                },
            )
            .await?;
            ensure_success(&response, request.config)?;
            request
                .config
                .transform_delete_skill_response(&response.body)
                .map_err(SkillsHandlerError::from)
        }
    }
}

impl SkillsListHttpHandler for HttpSkills {
    fn list_skills(
        &self,
        request: ListSkillsHttpRequest<'_>,
    ) -> impl Future<Output = Result<ListSkillsResponse, SkillsHandlerError>> + Send {
        async move {
            let response = exchange(
                &self.client,
                UpstreamCall {
                    method: Method::GET,
                    url: request.url,
                    headers: request.headers,
                    query: request.query_params,
                    body: RequestBody::Empty,
                    deadline: Exchange {
                        after: GET_SKILLS_TIMEOUT,
                        timeout_status: None,
                    },
                },
            )
            .await?;
            ensure_success(&response, request.config)?;
            request
                .config
                .transform_list_skills_response(&response.body)
                .map_err(SkillsHandlerError::from)
        }
    }
}

// The create call, holding this call's blobs until the assembled body resolves its $file markers.
pub(super) struct CreateTransport {
    pub(super) client: ProxiedClient,
    pub(super) blobs: Mutex<Vec<Vec<u8>>>,
}

impl SkillsCreateHttpHandler for CreateTransport {
    fn create_skill(
        &self,
        request: CreateSkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        async move {
            let (headers, body) = create_body(&request, &self.blobs)?;
            let response = exchange(
                &self.client,
                UpstreamCall {
                    method: Method::POST,
                    url: request.url,
                    headers,
                    query: Vec::new(),
                    body,
                    deadline: Exchange {
                        after: request.timeout,
                        timeout_status: Some(408_u16),
                    },
                },
            )
            .await?;
            ensure_success(&response, request.config)?;
            request
                .config
                .transform_create_skill_response(&response.body)
                .map_err(SkillsHandlerError::from)
        }
    }
}

// LiteLLM sends multipart only when files is non-empty, and the body JSON otherwise.
fn create_body(
    request: &CreateSkillHttpRequest<'_>,
    blobs: &Mutex<Vec<Vec<u8>>>,
) -> Result<(HeaderMap, RequestBody), SkillsHandlerError> {
    let mut headers = request.headers.clone();
    let Some(files) = request
        .body
        .files
        .as_deref()
        .filter(|files| !files.is_empty())
    else {
        let encoded = serde_json::to_vec(request.body).map_err(|error| {
            provider_failure(
                500,
                format!("the request body could not be written: {error}"),
                true,
            )
        })?;
        return Ok((headers, RequestBody::Json(encoded)));
    };
    let mut owned = match blobs.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let resolved = convert::files(request.custom_llm_provider, files, owned.as_mut_slice())
        .map_err(|error| provider_failure(400, error.to_string(), false))?;
    drop(owned);
    headers.remove(CONTENT_TYPE);
    let parts = multipart::parts(request.body, resolved);
    let body = RequestBody::Multipart {
        boundary: Uuid::new_v4().to_string(),
        parts,
    };
    Ok((headers, body))
}

async fn exchange(
    client: &ProxiedClient,
    call: UpstreamCall,
) -> Result<UpstreamResponse, SkillsHandlerError> {
    let url = Url::parse(&call.url).map_err(|_invalid| {
        provider_failure(
            500,
            format!("the skills url {} is not a URL", call.url),
            true,
        )
    })?;
    let deadline = call.deadline;
    let request = OutboundRequest {
        method: call.method,
        url,
        query: call.query,
        headers: call.headers,
        body: call.body,
        deadline: deadline.after,
    };
    client
        .send_buffered(request)
        .await
        .map_err(|error| transport_failure(&error, deadline))
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

fn provider_failure(status_code: u16, message: String, synthesized: bool) -> SkillsHandlerError {
    SkillsHandlerError::Provider {
        status_code,
        message,
        status_code_is_synthesized: synthesized,
        body_code: None,
        body_message: None,
    }
}
