use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use url::Url;

use crate::api::responses_compact_http::body::{SignedBody, signed_body_kwargs};
use crate::api::responses_compact_http::service::{ClientKind, SharedSession};
use crate::api::vector_store_http_write::error::display_facts;

use super::super::config::{
    CreateConfig, CreateParams, CreateSignRequest, CreateTransformRequest, ExtraBodyRequest,
    Headers, Signed,
};
use super::super::convert::{FloatError, float_value};
use super::super::error::{CreateError, Stage, provider_error};
use super::hook::{self, Applied};
use super::{ClientSource, CreateCall, CreateDeps, CreateRequest, LogContext, Selected, Staged};

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
    if let Some(extra_headers) = call.extra_headers.filter(|supplied| !supplied.is_empty()) {
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

// The provider's transform, normalize and extra_body steps; the async handler awaits the off-loop transform
pub(super) async fn transformed_data<C>(
    config: &C,
    mode: ClientKind,
    call: &CreateCall<'_>,
    applied: &Applied,
    headers: &Headers,
) -> Result<Object, CreateError>
where
    C: CreateConfig,
{
    let transform = CreateTransformRequest {
        model: &applied.model,
        input: &applied.input,
        optional_params: &applied.optional_params,
        params: &applied.params,
        headers,
    };
    let transformed = match mode {
        ClientKind::Sync => config.transform_request(&transform),
        ClientKind::Asynchronous => config.transform_request_off_loop(&transform).await,
    }
    .map_err(|source| CreateError::Step {
        stage: Stage::Transform,
        source,
    })?;
    let mut data = config
        .normalize_request(transformed)
        .map_err(|source| CreateError::Step {
            stage: Stage::Normalize,
            source,
        })?;
    // transform_extra_body gets the normalized data and its result overwrites the matching keys, as {**data, **extra}
    let Some(extra_body) = call.extra_body.filter(|body| !body.is_empty()) else {
        return Ok(data);
    };
    let transformed = config
        .transform_extra_body(&ExtraBodyRequest {
            extra_body,
            request: &data,
            model: &applied.model,
            params: &applied.params,
        })
        .map_err(|source| CreateError::Step {
            stage: Stage::Normalize,
            source,
        })?;
    data.extend(transformed);
    Ok(data)
}

// stream is bool(optional stream) or bool(body stream); an exact fake_stream=true buffers and drops body.stream
pub(super) fn stream_flags(
    call: &CreateCall<'_>,
    optional_params: &Object,
    data: &mut Object,
) -> (bool, bool) {
    let requested = optional_params
        .get("stream")
        .is_some_and(PromptJson::is_truthy);
    let stream_requested = requested || data.get("stream").is_some_and(PromptJson::is_truthy);
    if stream_requested && matches!(call.fake_stream, Some(Json::Bool(true))) {
        data.shift_remove("stream");
        return (stream_requested, false);
    }
    (stream_requested, stream_requested)
}

// The async handler awaits sign_request_off_loop; the sync handler calls sign_request directly
async fn sign<C>(
    config: &C,
    mode: ClientKind,
    request: &CreateSignRequest<'_>,
) -> Result<Signed, CreateError>
where
    C: CreateConfig,
{
    match mode {
        ClientKind::Sync => config.sign_request(request),
        ClientKind::Asynchronous => config.sign_request_off_loop(request).await,
    }
    .map_err(|source| CreateError::Step {
        stage: Stage::Sign,
        source,
    })
}

// The signing step sees the final body; a signed byte string replaces it, then pre-call logs the request
pub(super) async fn wire<C>(
    config: &C,
    mode: ClientKind,
    call: &CreateCall<'_>,
    staged: &Staged,
    log: &mut LogContext<'_, '_>,
) -> Result<(CreateRequest, Duration), CreateError>
where
    C: CreateConfig,
{
    let signing = CreateSignRequest {
        headers: &staged.headers,
        optional_params: &staged.params,
        request_data: &staged.data,
        api_base: &staged.api_base,
        api_key: staged.applied.params.api_key.as_ref(),
        model: &staged.applied.model,
        stream: staged.effective_stream,
        fake_stream: call.fake_stream,
    };
    let signed = sign(config, mode, &signing).await?;
    let body = wire_body(&signed_body_kwargs(signed.body.as_deref(), &staged.data))?;
    hook::pre_call(
        &staged.applied.input,
        &staged.api_base,
        &signed.headers,
        &staged.data,
        log,
    )?;
    let timeout = timeout(call.timeout, &staged.applied.optional_params).map_err(|error| {
        CreateError::Provider(provider_error(&display_facts(error.to_string()), config))
    })?;
    let post = CreateRequest {
        method: Method::POST,
        url: endpoint_url(&staged.api_base)?,
        headers: wire_headers(&signed.headers)?,
        body,
        timeout,
    };
    Ok((post, timeout))
}

// timeout or float(optional_params.get("timeout", 0)): an explicit zero falls through to the optional map
pub(super) fn timeout(
    explicit: Option<f64>,
    optional_params: &Object,
) -> Result<Duration, FloatError> {
    let computed = match explicit.filter(|value| *value != 0.0_f64) {
        Some(value) => value,
        None => match optional_params.get("timeout") {
            Some(value) => float_value(value)?,
            None => 0.0_f64,
        },
    };
    if computed.is_infinite() && computed > 0.0_f64 {
        return Ok(Duration::MAX);
    }
    Ok(Duration::try_from_secs_f64(computed).unwrap_or(Duration::ZERO))
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
