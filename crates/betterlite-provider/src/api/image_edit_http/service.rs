use std::time::{Duration, Instant};

use betterlite_core::api::vector_store_http_write::error::handle_error;
use betterlite_core::api::vector_store_http_write::service::{
    ClientSource, WriteRequest, WriteTransport,
};
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageParams, ImageResponse,
};
use http::Method;
use uuid::Uuid;

use super::error::{ImageEditHttpError, RequestError};
use super::facts::{self, Failure, Sent};
use super::request::{self, Logged};
use crate::api::image_contracts::edit::{
    BaseImageEditConfig, ImageEditEnvironment, ImageEditReply, ImageEditRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

// One image edit call's inputs: the fields the transforms read, the caller's headers and the request's timeout
pub struct ImageEditCall<'call> {
    pub model: &'call str,
    pub prompt: Option<&'call str>,
    pub image: Option<&'call ImageFile>,
    pub optional_params: &'call ImageParams,
    pub litellm_params: &'call ImageParams,
    pub extra_headers: Option<&'call ImageHeaders>,
    pub timeout: Duration,
}

pub struct ImageEditDeps<'deps, C: BaseImageEditConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct ImageEditOutcome {
    pub response: ImageResponse,
    pub provider_headers: ProviderHeaderProjection,
}

// The front half both handlers share: client selection, the environment headers and their explicit override, and the resolved URL the transform runs against
pub(super) fn prepare<'borrow, 'call, C, S>(
    mode: ClientKind,
    deps: &'borrow ImageEditDeps<'borrow, C, S>,
    call: &'borrow ImageEditCall<'call>,
) -> Result<Prepared<'borrow, 'call, C, S::Client>, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    S: ClientSource<Client: Clone>,
{
    let fields = request::fields(call.litellm_params)?;
    let caller = request::caller_headers(call.optional_params)?;
    let client = match deps.supplied.as_ref().filter(|client| client.kind == mode) {
        Some(supplied) => supplied.transport.clone(),
        None => deps.source.client(fields.ssl_verify)?,
    };
    let validated = deps.config.validate_environment(ImageEditEnvironment {
        headers: &caller,
        model: call.model,
        api_key: fields.api_key.as_ref(),
        litellm_params: Some(call.litellm_params),
        api_base: fields.api_base.as_deref(),
    });
    // The explicit map overwrites what the environment hook returned
    let headers = request::merge(validated, call.extra_headers);
    let resolved = deps
        .config
        .get_complete_url(call.model, fields.api_base.as_deref(), call.litellm_params)
        .map_err(ImageEditHttpError::Config)?;
    Ok(Prepared {
        config: deps.config,
        call,
        client,
        headers,
        resolved,
        mode,
    })
}

pub(super) struct Prepared<'borrow, 'call, C: BaseImageEditConfig, Client> {
    config: &'borrow C,
    call: &'borrow ImageEditCall<'call>,
    client: Client,
    headers: ImageHeaders,
    resolved: String,
    mode: ClientKind,
}

impl<C: BaseImageEditConfig, Client: WriteTransport> Prepared<'_, '_, C, Client> {
    pub(super) fn request(&self) -> ImageEditRequest<'_> {
        ImageEditRequest {
            model: self.call.model,
            prompt: self.call.prompt,
            image: self.call.image,
            optional_params: self.call.optional_params.clone(),
            litellm_params: self.call.litellm_params,
            headers: self.headers.clone(),
        }
    }

    // The finalizer over the transform's exact output, the pre-call, the wire request, then the send and the reply's transformation
    pub(super) async fn finish(
        self,
        transformed: Result<(ImageParams, ImageFiles), C::Error>,
        log: &mut LogContext<'_, '_>,
    ) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>> {
        let (data, files) = transformed.map_err(ImageEditHttpError::Config)?;
        let finalized = self
            .config
            .finalize_image_edit_request_data(data, &self.resolved);
        let logged = Logged {
            prompt: self.call.prompt,
            api_base: &self.resolved,
            headers: &self.headers,
            data: &finalized,
            files: &files,
        };
        request::log_pre_call(logged, self.mode, log)?;
        let body = if self.config.use_multipart_form_data() {
            RequestBody::Multipart {
                boundary: Uuid::new_v4().to_string(),
                parts: request::multipart_parts(&finalized, &files)?,
            }
        } else {
            RequestBody::Json(serde_json::to_vec(&finalized).map_err(RequestError::Encode)?)
        };
        let request = WriteRequest {
            method: Method::POST,
            url: request::endpoint(&self.resolved)?,
            headers: request::wire_headers(&self.headers)?,
            body,
            timeout: Some(self.call.timeout),
        };
        let model = self.call.model;
        send(self.config, &self.client, request, self.mode, model).await
    }
}

// One send, no retry: a transport failure becomes a provider error, and a status of 400 or more becomes one too
async fn send<C, Client>(
    config: &C,
    client: &Client,
    request: WriteRequest,
    mode: ClientKind,
    model: &str,
) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    Client: WriteTransport,
{
    let timeout = request.timeout.unwrap_or_default();
    let started = Instant::now();
    let reply = client.send(request).await;
    let sent = Sent {
        mode,
        timeout,
        elapsed: started.elapsed(),
    };
    let answer = match reply {
        Ok(answer) => answer,
        Err(error) => return Err(provider(config, &facts::transport_failure(&error, sent))),
    };
    if answer.status >= 400 {
        return Err(provider(config, &facts::reply_failure(&answer)));
    }
    let raw = facts::image_reply(&answer);
    let response = config
        .transform_image_edit_response(ImageEditReply {
            model,
            raw_response: &raw,
        })
        .map_err(ImageEditHttpError::Config)?;
    let provider_headers = set_provider_response_headers(
        &AdditionalHeaders::default(),
        &facts::provider_headers(&answer.headers),
    );
    Ok(ImageEditOutcome {
        response,
        provider_headers,
    })
}

// The config's hook decides the class; the shared converter decides the status, the message and whether the status was synthesized
fn provider<C>(config: &C, failure: &Failure) -> ImageEditHttpError<C::Error>
where
    C: BaseImageEditConfig,
{
    let base = handle_error(&failure.facts, None);
    let mut error = config.get_error_class(&base.message, base.status_code, &failure.headers);
    if base.status_code_is_synthesized {
        error.status_code_is_synthesized = true;
    }
    ImageEditHttpError::Provider(Box::new(error))
}
