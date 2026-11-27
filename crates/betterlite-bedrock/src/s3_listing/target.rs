use std::collections::BTreeMap;

use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
use betterlite_context::aws_partition::get_aws_partition;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::{RawValue, to_raw_value};

use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_settings::{AUTHENTICATION_PARAMS, AuthError, AuthParams, pop_aws_auth_params};
use crate::request_policy::{Params, RequestValue};
use crate::signing::target::RegionInputs;

use super::S3ListingError;

const S3_REGION_PARAM: &str = "s3_region_name";
const AWS_REGION_PARAM: &str = "aws_region_name";
const S3_ENDPOINT_URL_PARAM: &str = "s3_endpoint_url";
const S3_ACCESS_KEY_ID_PARAM: &str = "s3_access_key_id";
const S3_SECRET_ACCESS_KEY_PARAM: &str = "s3_secret_access_key";

// no Debug: auth carries the caller's AWS keys
#[derive(Clone)]
pub struct S3RequestTarget {
    pub endpoint_url: String,
    pub aws_region_name: String,
    pub auth: AuthParams,
    pub s3: AwsS3Settings,
}

fn text_field(params: &Params, field: &'static str) -> Result<Option<String>, S3ListingError> {
    match params.get(field) {
        None => Ok(None),
        Some(RequestValue::Text(text)) => Ok(Some(text.clone())),
        Some(RequestValue::Secret(secret)) => Ok(Some(secret.expose_secret().to_owned())),
        Some(RequestValue::Raw(raw)) => {
            serde_json::from_str::<Option<String>>(raw).map_err(|source| S3ListingError::Field {
                field,
                source: Some(source),
            })
        }
        Some(RequestValue::Mapping(_)) => Err(S3ListingError::Field {
            field,
            source: None,
        }),
    }
}

fn auth_value(field: &'static str, value: &RequestValue) -> Result<Box<RawValue>, S3ListingError> {
    let raw = match value {
        RequestValue::Text(text) => to_raw_value(text),
        RequestValue::Secret(secret) => to_raw_value(secret.expose_secret()),
        RequestValue::Raw(raw) => RawValue::from_string(raw.clone()),
        RequestValue::Mapping(_) => {
            return Err(S3ListingError::Field {
                field,
                source: None,
            });
        }
    };
    raw.map_err(|source| S3ListingError::Field {
        field,
        source: Some(source),
    })
}

// the aws_* parameters go through B590's shared AuthParams parser, session tags included
fn auth_params(params: &Params) -> Result<AuthParams, S3ListingError> {
    let mut raw = BTreeMap::new();
    for field in AUTHENTICATION_PARAMS {
        if matches!(field, AWS_REGION_PARAM | "aws_bedrock_runtime_endpoint") {
            continue;
        }
        let Some(value) = params.get(field) else {
            continue;
        };
        raw.insert(field.to_owned(), auth_value(field, value)?);
    }
    pop_aws_auth_params(&mut raw).map_err(|error| match error {
        AuthError::Field { field, source } => S3ListingError::Field {
            field,
            source: Some(source),
        },
        AuthError::SessionTags => S3ListingError::Field {
            field: "aws_session_tags",
            source: None,
        },
        AuthError::Parameters(source) => S3ListingError::Field {
            field: "aws_session_tags",
            source: Some(source),
        },
    })
}

// {**litellm_params, **optional_params}: unlike B644's upload path the request side wins here
fn request_region(
    params: &Params,
    region_inputs: RegionInputs<'_>,
) -> Result<String, S3ListingError> {
    let s3_region = text_field(params, S3_REGION_PARAM)?;
    let aws_region = text_field(params, AWS_REGION_PARAM)?;
    let preference = s3_region.filter(|region| !region.is_empty()).or(aws_region);
    // the preference travels as _get_aws_region_name's aws_region_name with an empty model
    let region = llm_region_name(LlmRegion {
        explicit: preference.as_deref(),
        model: Some(""),
        model_id: None,
        environment_region_name: region_inputs.environment_region_name,
        environment_region: region_inputs.environment_region,
        session_region: region_inputs.session_region,
    })?;
    Ok(region)
}

fn endpoint_url(params: &Params, aws_region_name: &str) -> Result<String, S3ListingError> {
    let configured = text_field(params, S3_ENDPOINT_URL_PARAM)?;
    let endpoint = configured.filter(|url| !url.is_empty()).unwrap_or_else(|| {
        format!(
            "https://s3.{aws_region_name}.{}",
            get_aws_partition(Some(aws_region_name)).dns_suffix
        )
    });
    Ok(endpoint.trim_end_matches('/').to_owned())
}

pub fn s3_request_target(
    optional_params: &Params,
    litellm_params: &Params,
    region_inputs: RegionInputs<'_>,
) -> Result<S3RequestTarget, S3ListingError> {
    let mut params = litellm_params.clone();
    params.extend(optional_params.clone());
    let aws_region_name = request_region(&params, region_inputs)?;
    let endpoint = endpoint_url(&params, &aws_region_name)?;
    let s3 = AwsS3Settings {
        access_key_id: text_field(&params, S3_ACCESS_KEY_ID_PARAM)?.map(SecretString::from),
        secret_access_key: text_field(&params, S3_SECRET_ACCESS_KEY_PARAM)?.map(SecretString::from),
    };
    let auth = auth_params(&params)?;
    Ok(S3RequestTarget {
        endpoint_url: endpoint,
        aws_region_name,
        auth,
        s3,
    })
}
