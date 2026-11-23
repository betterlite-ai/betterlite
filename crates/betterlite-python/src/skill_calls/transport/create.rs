use std::future::Future;
use std::sync::Mutex;

use betterlite_anthropic::skills::wire::Skill;
use betterlite_core::api::skills::create::handler::{
    CreateSkillHttpRequest, SkillsCreateHttpHandler,
};
use betterlite_core::api::skills::get_delete::handler::SkillsHandlerError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::RequestBody;
use http::header::{CONTENT_TYPE, HeaderMap};
use uuid::Uuid;

use crate::skill_calls::convert;
use crate::skill_calls::transport::{
    Call, Exchange, Transform, answered, multipart, provider_failure,
};

// The create call, holding this call's blobs until the assembled body resolves its $file markers.
pub(in crate::skill_calls) struct CreateTransport {
    pub(in crate::skill_calls) client: ProxiedClient,
    pub(in crate::skill_calls) blobs: Mutex<Vec<Vec<u8>>>,
}

impl SkillsCreateHttpHandler for CreateTransport {
    fn create_skill(
        &self,
        request: CreateSkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send {
        async move {
            let call = create_call(request, &self.blobs)?;
            answered(&self.client, call, CREATE_SKILL).await
        }
    }
}

const CREATE_SKILL: Transform<Skill> = |config, body| config.transform_create_skill_response(body);

fn create_call<'request>(
    request: CreateSkillHttpRequest<'request>,
    blobs: &Mutex<Vec<Vec<u8>>>,
) -> Result<Call<'request>, SkillsHandlerError> {
    let config = request.config;
    let (headers, body) = create_body(&request, blobs)?;
    let exchange = Exchange::create(request.timeout);
    Ok(Call {
        url: request.url,
        headers,
        query: Vec::new(),
        body,
        config,
        exchange,
    })
}

// LiteLLM sends multipart only when files is non-empty, and the body JSON otherwise.
pub(super) fn create_body(
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
