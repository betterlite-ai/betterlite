use super::DecodedOcrResponse;
use crate::json::{Json, Object};
use crate::ocr_error::{OcrError, OcrParamsError};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::str;
pub fn response_format(optional_params: &Object) -> Result<RequestFormat, OcrError> {
    match optional_params.get("req_format") {
        None | Some(Json::Null) => Ok(RequestFormat::Litellm),
        Some(Json::Text(format)) if format == "native" => Ok(RequestFormat::Native),
        Some(Json::Text(format)) if format == "litellm" => Ok(RequestFormat::Litellm),
        Some(_) => Err(OcrError::RequestFormat),
    }
}
pub fn to_json<T>(value: &T) -> Result<Json, OcrError>
where
    T: Serialize,
{
    let text =
        serde_json::to_string(value).map_err(|_source| OcrError::Params(OcrParamsError::Body))?;
    serde_json::from_str(&text).map_err(|_source| OcrError::Params(OcrParamsError::Body))
}
#[derive(Clone, Copy)]
pub enum OcrFieldKind {
    Request,
    Response,
}
pub fn decode_value<T>(value: &Json, prefix: &str, kind: OcrFieldKind) -> Result<T, OcrError>
where
    T: DeserializeOwned,
{
    let text = serde_json::to_string(value).map_err(|_source| match kind {
        OcrFieldKind::Request => OcrError::RequestField {
            path: prefix.to_owned(),
        },
        OcrFieldKind::Response => OcrError::ResponseField {
            path: prefix.to_owned(),
        },
    })?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|source| match kind {
        OcrFieldKind::Request => OcrError::RequestField {
            path: format!("{prefix}.{}", source.path()),
        },
        OcrFieldKind::Response => OcrError::ResponseField {
            path: format!("{prefix}.{}", source.path()),
        },
    })
}
pub fn decode_response<T>(bytes: &[u8], native: bool) -> Result<DecodedOcrResponse<T>, OcrError>
where
    T: DeserializeOwned,
{
    let text = str::from_utf8(bytes).map_err(|_source| OcrError::ResponseField {
        path: "response".to_owned(),
    })?;
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let data = serde_path_to_error::deserialize(&mut deserializer).map_err(|source| {
        OcrError::ResponseField {
            path: source.path().to_string(),
        }
    })?;
    deserializer
        .end()
        .map_err(|_source| OcrError::ResponseField {
            path: "response".to_owned(),
        })?;
    let native_json = if native {
        let object: Object =
            serde_json::from_str(text).map_err(|_source| OcrError::ResponseField {
                path: "response".to_owned(),
            })?;
        Some(Json::Object(object))
    } else {
        None
    };
    Ok(DecodedOcrResponse {
        data,
        native: native_json,
        text: text.to_owned(),
    })
}
pub fn decode_and_normalize_response<T, F>(
    model: &str,
    raw_response: &[u8],
    request_format: RequestFormat,
    normalize: F,
) -> Result<OCRResponse, OcrError>
where
    T: DeserializeOwned,
    F: FnOnce(&str, T) -> Result<OCRResponse, OcrError>,
{
    let decoded = decode_response(raw_response, request_format == RequestFormat::Native)?;
    let mut response = normalize(model, decoded.data)?;
    response.provider_native_response = decoded.native;
    Ok(response)
}
pub fn compose_body<B>(params: &Object, body: &B, consumed: &[&str]) -> Result<Json, OcrError>
where
    B: Serialize,
{
    let Json::Object(mut fields) = to_json(body)? else {
        return Err(OcrError::Params(OcrParamsError::Body));
    };
    let overrides: Option<Object> = match params.get("extra_body") {
        None | Some(Json::Null) => None,
        Some(Json::Object(overrides)) => Some(overrides.clone()),
        Some(_) => return Err(OcrError::Params(OcrParamsError::ExtraBody)),
    };
    for (name, value) in params {
        if consumed.contains(&name.as_str()) {
            continue;
        }
        if is_control_param(name) || matches!(name.as_str(), "model" | "extra_body") {
            continue;
        }
        let _previous = fields.insert(name.clone(), value.clone());
    }
    for (name, value) in overrides.into_iter().flatten() {
        if is_control_param(&name) || matches!(name.as_str(), "model" | "extra_body") {
            continue;
        }
        let _previous = fields.insert(name, value);
    }
    Ok(Json::Object(fields))
}
fn is_control_param(name: &str) -> bool {
    matches!(
        name,
        "api_key"
            | "api_base"
            | "custom_llm_provider"
            | "extra_headers"
            | "timeout"
            | "timeout_seconds"
            | "request_timeout"
            | "max_retries"
            | "req_format"
            | "max_response_bytes"
            | "azure_ad_token"
            | "azure_ad_token_provider"
            | "tenant_id"
            | "client_id"
            | "client_secret"
            | "azure_scope"
            | "azure_authority_host"
            | "azure_credential"
            | "azure_federated_token_file"
            | "enable_azure_ad_token_refresh"
            | "vertex_credentials"
            | "vertex_ai_credentials"
            | "vertex_project"
            | "vertex_ai_project"
            | "vertex_location"
            | "vertex_ai_location"
            | "aws_access_key_id"
            | "aws_secret_access_key"
            | "aws_session_token"
            | "aws_region_name"
            | "aws_session_name"
            | "aws_profile_name"
            | "aws_role_name"
            | "aws_web_identity_token"
            | "aws_sts_endpoint"
            | "aws_external_id"
            | "aws_bedrock_runtime_endpoint"
    )
}
