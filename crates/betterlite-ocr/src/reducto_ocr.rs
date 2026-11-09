mod numbers;
pub mod request;
mod response;
mod upload;
mod url;

use std::future::{Future, ready};

use crate::json::{Json, Object};
use crate::ocr_config::decode::{compose_body, decode_and_normalize_response};
use crate::ocr_config::{OcrProviderConfig, OcrRequestContext, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::{CallHooks, OcrClient, build_http_request, guardrail_document};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use request::{
    ReductoLegacyOptions, ReductoLegacyRequest, ReductoV3Request, reducto_id, resolve_headers,
    select_params,
};
use response::normalize_response;
use upload::ensure_file_id;
use url::complete_endpoint_url;
use uuid::Uuid;

const REDUCTO_API_BASE: &str = "https://platform.reducto.ai";
pub(crate) const REDUCTO_API_KEY_ENV: &str = "REDUCTO_API_KEY";
const REDUCTO_ID_PREFIX: &str = "reducto://";

// the id source is injected so a test or a caller can pin the multipart boundary
pub type Boundaries = fn() -> Uuid;

// both protocols share the auth, URL and upload flow; a config only picks its options and body
pub trait ReductoProviderBody: Send + Sync + Sized + 'static {
    type Request: serde::Serialize + Send;
    const SUPPORTED: &'static [&'static str];
    fn body(input: String, params: &Object) -> Self::Request;

    fn boundaries(&self) -> Boundaries;
}

async fn uploaded_request<Config>(
    config: &Config,
    document: OcrDocument,
    transform: OcrTransform<'_, Object>,
    context: OcrRequestContext<'_>,
) -> Result<Config::Request, OcrError>
where
    Config: ReductoProviderBody,
{
    let input = ensure_file_id(
        document.source(),
        transform.headers,
        context,
        config.boundaries(),
    )
    .await?;
    Ok(Config::body(input, transform.optional_params))
}

impl<Config> OcrProviderConfig for Config
where
    Config: ReductoProviderBody,
{
    type OcrParams = Object;
    type ProviderRequest = Config::Request;
    type Environment = Vec<(String, String)>;

    fn secret_names(&self) -> Vec<&'static str> {
        vec![REDUCTO_API_KEY_ENV]
    }

    fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
        Config::SUPPORTED
    }

    fn map_ocr_params(
        &self,
        non_default_params: &Object,
        model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        Ok(select_params(
            non_default_params,
            self.get_supported_ocr_params(model),
        ))
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        _client: &OcrClient,
    ) -> impl Future<Output = Result<Self::Environment, OcrError>> + Send {
        ready(resolve_headers(request))
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Self::OcrParams,
        _environment: &Self::Environment,
    ) -> Result<String, OcrError> {
        complete_endpoint_url(request.connection.api_base.as_deref(), "parse")
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        Ok(Self::body(
            reducto_id(document.source())?,
            transform.optional_params,
        ))
    }

    fn async_transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
        context: OcrRequestContext<'_>,
    ) -> impl Future<Output = Result<Self::ProviderRequest, OcrError>> + Send {
        uploaded_request(self, document, transform, context)
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        decode_and_normalize_response(model, raw_response, request_format, normalize_response)
    }

    fn prepare_request(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
        hooks: &dyn CallHooks,
    ) -> impl Future<Output = Result<OutboundRequest, OcrError>> + Send {
        prepare_upload_request(self, request, client, hooks)
    }
}

#[derive(Clone, Debug)]
pub struct ReductoParseV3Config {
    boundaries: Boundaries,
}

impl Default for ReductoParseV3Config {
    fn default() -> Self {
        Self {
            boundaries: Uuid::new_v4,
        }
    }
}

impl ReductoProviderBody for ReductoParseV3Config {
    type Request = ReductoV3Request;
    const SUPPORTED: &'static [&'static str] = &["formatting", "retrieval", "settings"];

    fn body(input: String, params: &Object) -> ReductoV3Request {
        let mut fields = params.clone();
        let _previous = fields.insert("input".to_owned(), Json::Text(input));
        ReductoV3Request(fields)
    }

    fn boundaries(&self) -> Boundaries {
        self.boundaries
    }
}

#[derive(Clone, Debug)]
pub struct ReductoParseLegacyConfig {
    boundaries: Boundaries,
}

impl Default for ReductoParseLegacyConfig {
    fn default() -> Self {
        Self {
            boundaries: Uuid::new_v4,
        }
    }
}

impl ReductoProviderBody for ReductoParseLegacyConfig {
    type Request = ReductoLegacyRequest;
    const SUPPORTED: &'static [&'static str] = &["enhance"];

    fn body(input: String, params: &Object) -> ReductoLegacyRequest {
        let options = params
            .get("enhance")
            .filter(|value| !matches!(value, Json::Null))
            .map(|enhance| ReductoLegacyOptions {
                enhance: enhance.clone(),
            });
        ReductoLegacyRequest {
            document_url: input,
            options,
        }
    }

    fn boundaries(&self) -> Boundaries {
        self.boundaries
    }
}

async fn prepare_upload_request<Config>(
    config: &Config,
    request: &PreparedOcrRequest,
    client: &OcrClient,
    hooks: &dyn CallHooks,
) -> Result<OutboundRequest, OcrError>
where
    Config: OcrProviderConfig<Environment = Vec<(String, String)>>,
{
    let params = config.map_ocr_params(&request.optional_params, &request.model)?;
    let environment = config.validate_environment(request, client).await?;
    let url = config.get_complete_url(request, &params, &environment)?;
    let (document, headers) = guardrail_document(request, &url, &environment, hooks).await?;
    let body = config
        .async_transform_ocr_request(
            document,
            OcrTransform {
                model: &request.model,
                optional_params: &params,
                headers: &headers,
            },
            OcrRequestContext {
                client,
                connection: &request.connection,
            },
        )
        .await?;
    let composed = compose_body(
        &request.optional_params,
        &body,
        config.get_supported_ocr_params(&request.model),
    )?;
    build_http_request(request, &url, headers, &composed)
}

mod tests;
