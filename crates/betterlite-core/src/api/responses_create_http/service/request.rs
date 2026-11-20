use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_prompts::arguments::json::Object;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use url::Url;

use crate::api::responses_compact_http::body::SignedBody;
use crate::api::responses_compact_http::error::Stage;
use crate::api::responses_compact_http::service::{ClientKind, SharedSession};

use super::super::config::{
    CreateConfig, CreateParams, CreateSignRequest, CreateTransformRequest, ExtraBodyRequest,
    Headers, Signed,
};
use super::super::convert::python_float;
use super::super::error::CreateError;
use super::hook::{self, Applied};
use super::{ClientSource, CreateCall, CreateDeps, Selected};

// The supplied client is reused only when its class matches the handler's, as isinstance does in LiteLLM
pub(super) fn select_client<'deps, C, S>(
    deps: &'deps CreateDeps<'_, C, S>,
    mode: ClientKind,
    params: &CreateParams,
    shared_session: Option<SharedSession>,
) -> Result<Selected<'deps, S::Client>, CreateError>
where
    C: CreateConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    let session = match mode {
        ClientKind::Sync => None,
        ClientKind::Asynchronous => shared_session,
    };
    if mode == ClientKind::Asynchronous {
        tracing::debug!(
            "Creating HTTP client for responses API with shared_session: {}",
            session.map_or_else(|| String::from("None"), |value| value.0.to_string()),
        );
    }
    Ok(Selected::Created(
        deps.source.client(params.ssl_verify, session)?,
    ))
}

// validate_environment reads the optional map's extra_headers, then the separate argument overlays them
pub(super) fn validated_environment<C>(
    config: &C,
    call: &CreateCall<'_>,
    applied: &Applied,
) -> Result<(Headers, String), CreateError>
where
    C: CreateConfig,
{
    let optional = hook::optional_headers(&applied.optional_params)?;
    let mut headers = config
        .validate_environment(optional, &applied.model, &applied.params)
        .map_err(|source| CreateError::Step {
            stage: Stage::Validate,
            source,
        })?;
    if let Some(extra_headers) = call.extra_headers.filter(|headers| !headers.is_empty()) {
        headers.extend(extra_headers.clone());
    }
    let api_base = config
        .complete_url(&applied.params)
        .map_err(|source| CreateError::Step {
            stage: Stage::Url,
            source,
        })?;
    Ok((headers, api_base))
}

// The async handler awaits async_transform_responses_api_request; the sync one calls the sync form
pub(super) async fn transform<C>(
    config: &C,
    mode: ClientKind,
    applied: &Applied,
    headers: &Headers,
) -> Result<Object, CreateError>
where
    C: CreateConfig,
{
    let request = CreateTransformRequest {
        model: &applied.model,
        input: &applied.input,
        optional_params: &applied.optional_params,
        params: &applied.params,
        headers,
    };
    let transformed = match mode {
        ClientKind::Sync => config.transform_request(&request),
        ClientKind::Asynchronous => config.transform_request_off_loop(&request).await,
    };
    transformed.map_err(|source| CreateError::Step {
        stage: Stage::Transform,
        source,
    })
}

pub(super) fn normalize<C>(config: &C, data: Object) -> Result<Object, CreateError>
where
    C: CreateConfig,
{
    config
        .normalize_request(data)
        .map_err(|source| CreateError::Step {
            stage: Stage::Normalize,
            source,
        })
}

pub(super) fn extra_body<C>(
    config: &C,
    extra_body: &Object,
    request: &Object,
    applied: &Applied,
) -> Result<Object, CreateError>
where
    C: CreateConfig,
{
    config
        .transform_extra_body(&ExtraBodyRequest {
            extra_body,
            request,
            model: &applied.model,
            params: &applied.params,
        })
        .map_err(|source| CreateError::Step {
            stage: Stage::Normalize,
            source,
        })
}

// The async handler awaits sign_request_off_loop_if_aws; the sync handler calls sign_request directly
pub(super) async fn sign<C>(
    config: &C,
    mode: ClientKind,
    request: &CreateSignRequest<'_>,
) -> Result<Signed, CreateError>
where
    C: CreateConfig,
{
    let signed = match mode {
        ClientKind::Sync => config.sign_request(request),
        ClientKind::Asynchronous => config.sign_request_off_loop(request).await,
    };
    signed.map_err(|source| CreateError::Step {
        stage: Stage::Sign,
        source,
    })
}

// timeout or float(optional_params.get("timeout", 0)): an explicit zero falls through to the optional map
pub(super) fn timeout(explicit: Option<f64>, optional_params: &Object) -> Result<Duration, String> {
    let computed = match explicit.filter(|value| *value != 0.0_f64) {
        Some(value) => value,
        None => match optional_params.get("timeout") {
            Some(value) => python_float(value)?,
            None => 0.0_f64,
        },
    };
    Ok(deadline(computed))
}

fn deadline(seconds: f64) -> Duration {
    if seconds.is_infinite() && seconds > 0.0_f64 {
        return Duration::MAX;
    }
    Duration::try_from_secs_f64(seconds).unwrap_or(Duration::ZERO)
}

pub(super) fn endpoint_url(text: &str) -> Result<Url, CreateError> {
    Url::parse(text).map_err(|source| CreateError::RequestUrl {
        url: text.to_owned(),
        source,
    })
}

pub(super) fn wire_headers(headers: &Headers) -> Result<HttpHeaders, CreateError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| CreateError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| CreateError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// A signed byte string is the whole body, even empty; otherwise the normalized data is serialized as JSON
pub(super) fn wire_body(body: &SignedBody<'_>) -> Result<RequestBody, CreateError> {
    match *body {
        SignedBody::Data(bytes) => Ok(RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/octet-stream"),
            data: bytes.to_vec(),
        }),
        SignedBody::Json(data) => serde_json::to_vec(data)
            .map(RequestBody::Json)
            .map_err(CreateError::Encode),
    }
}
