use std::str;

use betterlite_openai::embedding::{
    EmbeddingInput, EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use http::HeaderMap;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_json::error::Category;

use crate::oci_common::base_url::{OCI_API_VERSION, get_oci_base_url};
use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::environment::{OciEnvironment, validate_oci_environment};
use crate::oci_common::{OciError, OciServingMode};

mod tests;

pub const OCI_EMBED_BATCH_LIMIT: usize = 96;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["dimensions"];

const DEFAULT_SERVING_MODE: &str = "ON_DEMAND";
const DEFAULT_TRUNCATE: &str = "END";

const INPUT_TYPE_NAMES: [(&str, &str); 4] = [
    ("search_document", "SEARCH_DOCUMENT"),
    ("search_query", "SEARCH_QUERY"),
    ("classification", "CLASSIFICATION"),
    ("clustering", "CLUSTERING"),
];

const MISSING_CREDENTIALS: &str = "Missing required parameters: oci_user, oci_fingerprint, oci_tenancy, oci_compartment_id and at least one of oci_key or oci_key_file. These can be supplied via optional_params or via OCI_USER, OCI_FINGERPRINT, OCI_TENANCY, OCI_COMPARTMENT_ID, OCI_KEY_FILE environment variables. Alternatively, provide an oci_signer object from the OCI SDK.";

const MISSING_COMPARTMENT: &str = "oci_compartment_id is required for OCI embedding requests. Pass it as optional_params or set the OCI_COMPARTMENT_ID env var.";

const TOKEN_ARRAY_INPUT: &str = "OCI embedText does not support token-array inputs. Convert token lists to strings before calling embedding().";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciEmbedParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<i64>,
    #[serde(
        default,
        rename = "outputDimensions",
        skip_serializing_if = "Option::is_none"
    )]
    pub output_dimensions: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oci_serving_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oci_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciEmbedRequest {
    pub compartment_id: String,
    pub serving_mode: OciServingMode,
    pub inputs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    pub truncate: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dimensions: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciEmbedResponse {
    #[serde(default)]
    pub id: Option<String>,
    pub embeddings: Vec<Vec<Number>>,
    pub model_id: String,
    pub model_version: String,
    #[serde(default)]
    pub input_text_token_counts: Option<Vec<u32>>,
    #[serde(default)]
    pub usage: Option<OciEmbedUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciEmbedUsage {
    pub prompt_tokens: u32,
    pub total_tokens: u32,
}

#[must_use]
pub const fn map_openai_params(
    non_default_params: &OciEmbedParams,
    mut optional_params: OciEmbedParams,
) -> OciEmbedParams {
    if let Some(dimensions) = non_default_params.dimensions {
        optional_params.output_dimensions = Some(dimensions);
    }
    optional_params
}

pub fn validate_environment(
    headers: &mut HeaderMap,
    credentials: &OciCredentials,
    environment: &OciEnvironment,
) -> Result<(), OciError> {
    let present = |value: Option<&str>| value.is_some_and(|text| !text.is_empty());
    let key_present = credentials
        .oci_key
        .as_ref()
        .is_some_and(|key| !key.expose_secret().is_empty())
        || present(credentials.oci_key_file.as_deref());
    let complete = present(credentials.oci_user.as_deref())
        && present(credentials.oci_fingerprint.as_deref())
        && present(credentials.oci_tenancy.as_deref())
        && present(credentials.oci_compartment_id.as_deref())
        && key_present;
    if !complete {
        return Err(OciError {
            status: 401,
            message: MISSING_CREDENTIALS.to_owned(),
        });
    }
    validate_oci_environment(headers, environment)
}

pub fn get_complete_url(
    credentials: &OciCredentials,
    api_base: Option<&str>,
) -> Result<String, OciError> {
    let base = get_oci_base_url(&credentials.oci_region, api_base)?;
    Ok(format!("{base}/{OCI_API_VERSION}/actions/embedText"))
}

pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &OciEmbedParams,
    credentials: &OciCredentials,
) -> Result<OciEmbedRequest, OciError> {
    let compartment_id = credentials
        .oci_compartment_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OciError {
            status: 400,
            message: MISSING_COMPARTMENT.to_owned(),
        })?;
    Ok(OciEmbedRequest {
        compartment_id: compartment_id.to_owned(),
        serving_mode: serving_mode(model, optional_params)?,
        inputs: embed_inputs(input)?,
        input_type: optional_params.input_type.as_deref().map(input_type_name),
        truncate: optional_params
            .truncate
            .clone()
            .unwrap_or_else(|| DEFAULT_TRUNCATE.to_owned()),
        output_dimensions: optional_params.output_dimensions,
    })
}

fn serving_mode(model: &str, optional_params: &OciEmbedParams) -> Result<OciServingMode, OciError> {
    let mode = optional_params
        .oci_serving_mode
        .as_deref()
        .unwrap_or(DEFAULT_SERVING_MODE)
        .to_uppercase();
    match mode.as_str() {
        "ON_DEMAND" => Ok(OciServingMode::OnDemand {
            model_id: model.to_owned(),
        }),
        "DEDICATED" => Ok(OciServingMode::Dedicated {
            endpoint_id: optional_params
                .oci_endpoint_id
                .as_deref()
                .unwrap_or(model)
                .to_owned(),
        }),
        _ => Err(OciError {
            status: 400,
            message: "oci_serving_mode must be 'ON_DEMAND' or 'DEDICATED'.".to_owned(),
        }),
    }
}

fn embed_inputs(input: &EmbeddingInput) -> Result<Vec<String>, OciError> {
    let texts = match input {
        EmbeddingInput::Text(text) => vec![text.clone()],
        EmbeddingInput::Texts(texts) => texts.clone(),
        EmbeddingInput::Tokens(tokens) => tokens.iter().map(u32::to_string).collect(),
        EmbeddingInput::TokenLists(_) => {
            return Err(OciError {
                status: 400,
                message: TOKEN_ARRAY_INPUT.to_owned(),
            });
        }
    };
    if texts.len() > OCI_EMBED_BATCH_LIMIT {
        return Err(OciError {
            status: 400,
            message: format!(
                "OCI embedText accepts at most {OCI_EMBED_BATCH_LIMIT} inputs per request (got {}). Batch your requests.",
                texts.len()
            ),
        });
    }
    Ok(texts)
}

fn input_type_name(value: &str) -> String {
    let lowered = value.to_lowercase();
    INPUT_TYPE_NAMES
        .iter()
        .find(|(name, _)| *name == lowered.as_str())
        .map_or_else(|| value.to_uppercase(), |(_, mapped)| (*mapped).to_owned())
}

pub fn decode_embed_response(status: u16, body: &[u8]) -> Result<EmbeddingResponse, OciError> {
    if status != 200 {
        return Err(OciError {
            status,
            message: body_text(body),
        });
    }
    let parsed: OciEmbedResponse = serde_json::from_slice(body).map_err(|source| {
        if source.classify() == Category::Data {
            OciError {
                status: 500,
                message: format!("OCI embed response does not match expected schema: {source}"),
            }
        } else {
            OciError {
                status,
                message: format!("Failed to parse OCI embed response as JSON: {source}"),
            }
        }
    })?;
    let usage = response_usage(&parsed)?;
    let row_count = parsed.embeddings.len();
    let mut data = Vec::with_capacity(row_count);
    for (position, embedding) in parsed.embeddings.into_iter().enumerate() {
        let index = u32::try_from(position).map_err(|_source| OciError {
            status: 500,
            message: "OCI embed response does not match expected schema: too many embedding rows"
                .to_owned(),
        })?;
        data.push(EmbeddingRecord {
            object: "embedding".to_owned(),
            embedding: EmbeddingVector::Floats(embedding),
            index,
        });
    }
    Ok(EmbeddingResponse {
        object: "list".to_owned(),
        data,
        model: parsed.model_id,
        usage,
    })
}

fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| {
            format!(
                "the response body holds {} bytes that are not UTF-8",
                body.len()
            )
        },
        str::to_owned,
    )
}

fn response_usage(parsed: &OciEmbedResponse) -> Result<EmbeddingUsage, OciError> {
    let (prompt_tokens, total_tokens) = match (&parsed.input_text_token_counts, parsed.usage) {
        (Some(counts), _) => {
            let total = token_count_sum(counts)?;
            (total, total)
        }
        (None, Some(usage)) => (usage.prompt_tokens, usage.total_tokens),
        (None, None) => (0, 0),
    };
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        completion_tokens_details: None,
        prompt_tokens_details: None,
        total_tokens,
    })
}

fn token_count_sum(counts: &[u32]) -> Result<u32, OciError> {
    let mut total = 0_u32;
    for count in counts {
        total = total.checked_add(*count).ok_or_else(|| OciError {
            status: 500,
            message: "OCI embed response does not match expected schema: the token counts overflow"
                .to_owned(),
        })?;
    }
    Ok(total)
}
