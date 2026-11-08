mod body;
mod request;
use crate::json::Json;
use crate::ocr_auth::{AwsRequestSigner, AwsSigningRequest, AzureTokenSource, VertexTokenSource};
use crate::ocr_config::decode::{
    OcrFieldKind, compose_body, decode_response, decode_value, response_format, to_json,
};
use crate::ocr_config::{
    DecodedOcrResponse, OcrEnvironment, OcrProviderConfig, OcrResponseContext, PreparedOcrRequest,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrError, OcrParamsError, OcrSecretError, OcrTransportError};
use crate::ocr_settings::OcrSettings;
use crate::response::OCRResponse;
use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::SystemResolver;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
pub struct OcrClient {
    pub provider_http: ProxiedClient,
    pub polling_http: ProxiedClient,
    pub document_http: ProxiedClient,
    pub document_fetcher: UserUrlFetcher<SystemResolver>,
    pub settings: OcrSettings,
    pub secrets: Arc<dyn SecretSource>,
    pub azure: Arc<dyn AzureTokenSource>,
    pub aws: Arc<dyn AwsRequestSigner>,
    pub vertex: Arc<dyn VertexTokenSource>,
}
pub trait SecretSource: Send + Sync {
    fn secret(&self, name: &str) -> Result<Option<SecretString>, OcrSecretError>;
}
#[derive(Clone, Debug)]
pub struct OcrWireRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Json,
}
#[async_trait]
pub trait CallHooks: Send + Sync {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError>;
    async fn response_received(&self, body: &[u8]) -> Result<(), OcrError>;
}
#[derive(Clone, Copy)]
pub struct OcrCallContext<'call> {
    pub environment: &'call dyn OcrEnvironment,
    pub hooks: &'call dyn CallHooks,
}
pub async fn ocr<C>(
    config: &C,
    client: &OcrClient,
    request: &PreparedOcrRequest,
    hooks: &dyn CallHooks,
) -> Result<OCRResponse, OcrError>
where
    C: OcrProviderConfig,
{
    let outbound = config.prepare_request(request, client, hooks).await?;
    let url = outbound.url.to_string();
    let headers = request::header_pairs(&outbound.headers);
    let response = client
        .provider_http
        .send(outbound)
        .await
        .map_err(request::transport_failure)?;
    if !response.response.status().is_success() {
        let response_headers = request::header_pairs(response.response.headers());
        return match read_response_bytes(response, request.connection.max_response_bytes).await {
            Err(OcrError::Transport(OcrTransportError::Http { status, body })) => {
                Err(config.get_error_class(body, status, response_headers))
            }
            Err(error) => Err(error),
            Ok(_) => Err(OcrError::InvalidResponse(
                "a failed provider answer carried no status".to_owned(),
            )),
        };
    }
    let context = OcrResponseContext {
        client,
        connection: &request.connection,
        hooks,
        request_format: response_format(&request.optional_params)?,
        url: &url,
        headers: &headers,
    };
    config
        .async_transform_ocr_response(&request.model, response, context)
        .await
}
pub async fn read_json_response<T>(
    response: StreamResponse,
    native: bool,
    max_response_bytes: usize,
) -> Result<DecodedOcrResponse<T>, OcrError>
where
    T: DeserializeOwned,
{
    let bytes = read_response_bytes(response, max_response_bytes).await?;
    let decoded = decode_response(&bytes, native)?;
    Ok(decoded)
}
pub async fn read_response_bytes(
    mut response: StreamResponse,
    limit: usize,
) -> Result<Vec<u8>, OcrError> {
    let status = response.response.status();
    let success = status.is_success();
    if success && body::declared_too_large(&response, limit) {
        return Err(OcrError::TooLarge { limit });
    }
    let bytes = body::read_body(&mut response, limit, success).await?;
    if !success {
        return Err(OcrError::Transport(OcrTransportError::Http {
            status: status.as_u16(),
            body: body::printable_body(&bytes),
        }));
    }
    Ok(bytes)
}
pub async fn transform_request_body<C, B>(
    config: &C,
    request: &PreparedOcrRequest,
    url: &str,
    context: OcrCallContext<'_>,
    body: B,
) -> Result<OutboundRequest, OcrError>
where
    C: OcrProviderConfig,
    B: Serialize,
{
    let (changed, wire_headers) = hooked_wire(config, request, url, context, body).await?;
    let bytes =
        serde_json::to_vec(&changed).map_err(|_source| OcrError::Params(OcrParamsError::Body))?;
    let mut headers = request::with_content_type(wire_headers);
    if let Some(signing) = context.environment.signer() {
        let signature = signing
            .signer
            .sign(&AwsSigningRequest {
                method: "POST",
                url,
                headers: &headers,
                body: &bytes,
                service: signing.service,
                region: signing.region,
                optional_params: &request.optional_params,
                secrets: &request.connection.secrets,
            })
            .await?;
        headers.extend(signature);
    }
    request::outbound(request, url, headers, bytes)
}
async fn hooked_wire<C, B>(
    config: &C,
    request: &PreparedOcrRequest,
    url: &str,
    context: OcrCallContext<'_>,
    body: B,
) -> Result<(Json, Vec<(String, String)>), OcrError>
where
    C: OcrProviderConfig,
    B: Serialize,
{
    let composed = compose_body(
        &request.optional_params,
        &body,
        config.get_supported_ocr_params(&request.model),
    )?;
    config.validate_request_body(&composed)?;
    let changed = context
        .hooks
        .before_provider_request(OcrWireRequest {
            url: url.to_owned(),
            headers: context.environment.headers().to_vec(),
            body: composed,
        })
        .await?;
    let Json::Object(_) = &changed.body else {
        return Err(OcrError::RequestField {
            path: "guardrail.body".to_owned(),
        });
    };
    config.validate_request_body(&changed.body)?;
    Ok((changed.body, changed.headers))
}
pub fn build_http_request<B>(
    request: &PreparedOcrRequest,
    url: &str,
    headers: Vec<(String, String)>,
    body: &B,
) -> Result<OutboundRequest, OcrError>
where
    B: Serialize,
{
    let bytes =
        serde_json::to_vec(body).map_err(|_source| OcrError::Params(OcrParamsError::Body))?;
    request::outbound(request, url, headers, bytes)
}
pub async fn guardrail_document(
    request: &PreparedOcrRequest,
    url: &str,
    headers: &[(String, String)],
    hooks: &dyn CallHooks,
) -> Result<(OcrDocument, Vec<(String, String)>), OcrError> {
    let body = to_json(&request.document).map_err(|_source| OcrError::RequestField {
        path: "document".to_owned(),
    })?;
    let changed = hooks
        .before_provider_request(OcrWireRequest {
            url: url.to_owned(),
            headers: headers.to_vec(),
            body,
        })
        .await?;
    let document = decode_value(&changed.body, "guardrail.document", OcrFieldKind::Request)?;
    Ok((document, changed.headers))
}
pub fn body_document(body: &Json) -> Result<OcrDocument, OcrError> {
    let Json::Object(fields) = body else {
        return Err(OcrError::RequestField {
            path: "body.document".to_owned(),
        });
    };
    let Some(Json::Object(document)) = fields.get("document") else {
        return Err(OcrError::RequestField {
            path: "body.document".to_owned(),
        });
    };
    let source = document
        .iter()
        .filter(|(name, _)| matches!(name.as_str(), "type" | "image_url" | "document_url"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    decode_value(
        &Json::Object(source),
        "body.document",
        OcrFieldKind::Request,
    )
}
pub(crate) mod tests;
