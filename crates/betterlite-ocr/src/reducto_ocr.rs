mod numbers;
pub mod request;
mod response;
mod upload;
mod url;

use std::future::{Future, ready};

use crate::json::Object;
use crate::ocr_config::decode::{compose_body, decode_and_normalize_response};
use crate::ocr_config::{OcrProviderConfig, OcrRequestContext, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::{CallHooks, OcrClient, build_http_request, guardrail_document};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use request::{
    ReductoLegacyRequest, ReductoV3Request, legacy_body, reducto_id, resolve_headers, select_params,
};
use response::normalize_response;
use upload::ensure_file_id;
use url::complete_endpoint_url;

const REDUCTO_API_BASE: &str = "https://platform.reducto.ai";
pub(crate) const REDUCTO_API_KEY_ENV: &str = "REDUCTO_API_KEY";
const REDUCTO_ID_PREFIX: &str = "reducto://";

#[derive(Clone, Debug)]
pub struct ReductoParseV3Config;

#[derive(Clone, Debug)]
pub struct ReductoParseLegacyConfig;

// both protocols share auth, URL and upload; only the supported options and body differ
macro_rules! reducto_provider {
    ($config:ident, $request:ty, $supported:expr, $body:item) => {
        impl $config {
            $body

            async fn uploaded_body(
                document: OcrDocument,
                transform: OcrTransform<'_, Object>,
                context: OcrRequestContext<'_>,
            ) -> Result<$request, OcrError> {
                let input = ensure_file_id(document.source(), transform.headers, context).await?;
                Ok(Self::body(input, transform.optional_params))
            }
        }

        impl OcrProviderConfig for $config {
            type OcrParams = Object;
            type ProviderRequest = $request;
            type Environment = Vec<(String, String)>;

            fn secret_names(&self) -> Vec<&'static str> {
                vec![REDUCTO_API_KEY_ENV]
            }

            fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
                $supported
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
                Ok(Self::body(reducto_id(document.source())?, transform.optional_params))
            }

            fn async_transform_ocr_request(
                &self,
                document: OcrDocument,
                transform: OcrTransform<'_, Self::OcrParams>,
                context: OcrRequestContext<'_>,
            ) -> impl Future<Output = Result<Self::ProviderRequest, OcrError>> + Send {
                Self::uploaded_body(document, transform, context)
            }

            fn transform_ocr_response(
                &self,
                model: &str,
                raw_response: &[u8],
                request_format: RequestFormat,
            ) -> Result<OCRResponse, OcrError> {
                decode_and_normalize_response(
                    model,
                    raw_response,
                    request_format,
                    normalize_response,
                )
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
    };
}

reducto_provider!(
    ReductoParseV3Config,
    ReductoV3Request,
    &["formatting", "retrieval", "settings"],
    fn body(input: String, params: &Object) -> ReductoV3Request {
        ReductoV3Request {
            input,
            params: params.clone(),
        }
    }
);

reducto_provider!(
    ReductoParseLegacyConfig,
    ReductoLegacyRequest,
    &["enhance"],
    fn body(input: String, params: &Object) -> ReductoLegacyRequest {
        legacy_body(input, params)
    }
);

async fn prepare_upload_request<C>(
    config: &C,
    request: &PreparedOcrRequest,
    client: &OcrClient,
    hooks: &dyn CallHooks,
) -> Result<OutboundRequest, OcrError>
where
    C: OcrProviderConfig<Environment = Vec<(String, String)>>,
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
