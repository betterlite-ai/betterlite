use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::response_resources::request::ResourceRequest;
use betterlite_prompts::arguments::json::Object;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use url::Url;

use super::super::body::SignedBody;
use super::super::config::{CompactConfig, CompactTransformRequest, Headers, SignRequest, Signed};
use super::super::error::{CompactError, Stage};
use super::{ClientKind, ClientSource, CompactCall, CompactDeps, Selected};

// The supplied client is reused only when its class matches the handler's, as isinstance does in LiteLLM
pub(super) fn select_client<'deps, C, S>(
    deps: &'deps CompactDeps<'_, C, S>,
    mode: ClientKind,
    call: &CompactCall<'_>,
) -> Result<Selected<'deps, S::Client>, CompactError>
where
    C: CompactConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    let shared_session = match mode {
        ClientKind::Sync => None,
        ClientKind::Asynchronous => call.shared_session,
    };
    if mode == ClientKind::Asynchronous {
        tracing::debug!(
            "Creating HTTP client for compact_response with shared_session: {}",
            shared_session.map_or_else(|| String::from("None"), |session| session.0.to_string()),
        );
    }
    Ok(Selected::Created(
        deps.source.client(call.params.ssl_verify, shared_session)?,
    ))
}

// validate_environment rewrites the caller's map in place, so a caller's Authorization does not survive it
pub(super) fn validated_environment<C>(
    config: &C,
    call: &CompactCall<'_>,
) -> Result<(Headers, String), CompactError>
where
    C: CompactConfig,
{
    let headers = config
        .validate_environment(
            call.extra_headers.cloned().unwrap_or_default(),
            call.model,
            call.params,
        )
        .map_err(|source| CompactError::Step {
            stage: Stage::Validate,
            source,
        })?;
    let api_base = config
        .complete_url(call.params)
        .map_err(|source| CompactError::Step {
            stage: Stage::Url,
            source,
        })?;
    Ok((headers, api_base))
}

pub(super) fn transform<C>(
    config: &C,
    call: &CompactCall<'_>,
    api_base: &str,
    headers: &Headers,
) -> Result<ResourceRequest, CompactError>
where
    C: CompactConfig,
{
    let request = CompactTransformRequest {
        model: call.model,
        input: call.input,
        optional_params: call.optional_params,
        api_base,
        params: call.params,
        headers,
    };
    match config.transform_compact_request(&request) {
        Ok(transformed) => Ok(transformed),
        Err(source) => Err(CompactError::Step {
            stage: Stage::Transform,
            source,
        }),
    }
}

pub(super) fn normalized_request<C>(config: &C, data: Object) -> Result<Object, CompactError>
where
    C: CompactConfig,
{
    config
        .normalize_request(data)
        .map_err(|source| CompactError::Step {
            stage: Stage::Normalize,
            source,
        })
}

// The async handler awaits sign_request_off_loop_if_aws; the sync handler calls sign_request directly
pub(super) async fn sign<C>(
    config: &C,
    mode: ClientKind,
    request: &SignRequest<'_>,
) -> Result<Signed, CompactError>
where
    C: CompactConfig,
{
    match mode {
        ClientKind::Sync => config.sign_request(request),
        ClientKind::Asynchronous => config.sign_request_off_loop(request).await,
    }
    .map_err(|source| CompactError::Step {
        stage: Stage::Sign,
        source,
    })
}

pub(super) fn endpoint_url(text: &str) -> Result<Url, CompactError> {
    Url::parse(text).map_err(|source| CompactError::RequestUrl {
        url: text.to_owned(),
        source,
    })
}

pub(super) fn wire_headers(headers: &Headers) -> Result<HttpHeaders, CompactError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| CompactError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| CompactError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// A signed byte string is the whole body; otherwise the normalized data is serialized as JSON
pub(super) fn wire_body(body: &SignedBody<'_>) -> Result<RequestBody, CompactError> {
    match *body {
        SignedBody::Data(bytes) => Ok(RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/octet-stream"),
            data: bytes.to_vec(),
        }),
        SignedBody::Json(data) => serde_json::to_vec(data)
            .map(RequestBody::Json)
            .map_err(CompactError::Encode),
    }
}
