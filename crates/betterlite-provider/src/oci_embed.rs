use betterlite_openai::embedding::EmbeddingInput;
use http::HeaderMap;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};

use crate::oci_common::base_url::{OCI_API_VERSION, get_oci_base_url};
use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::environment::{OciEnvironment, validate_oci_environment};
use crate::oci_common::{OciError, OciServingMode};

pub mod response;

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
    Ok(format!(
        "{}/{OCI_API_VERSION}/actions/embedText",
        get_oci_base_url(&credentials.oci_region, api_base)?
    ))
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
