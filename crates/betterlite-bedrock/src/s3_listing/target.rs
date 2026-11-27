use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
use betterlite_config::sections::bedrock_settings::SessionTagValue;
use betterlite_context::aws_partition::get_aws_partition;
use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_settings::AuthParams;
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

fn secret_field(
    params: &Params,
    field: &'static str,
) -> Result<Option<SecretString>, S3ListingError> {
    let text = text_field(params, field)?;
    Ok(text.map(SecretString::from))
}

fn session_tag_value(value: &RequestValue) -> SessionTagValue {
    match value {
        RequestValue::Text(text) => SessionTagValue::Text(text.clone()),
        RequestValue::Secret(secret) => SessionTagValue::Text(secret.expose_secret().to_owned()),
        RequestValue::Mapping(entries) => SessionTagValue::Mapping(
            entries
                .iter()
                .map(|(name, entry)| (name.clone(), session_tag_value(entry)))
                .collect(),
        ),
        RequestValue::Raw(raw) => serde_json::from_str(raw).unwrap_or(SessionTagValue::Null),
    }
}

fn session_tags(
    params: &Params,
    field: &'static str,
) -> Result<Option<SessionTagValue>, S3ListingError> {
    match params.get(field) {
        None => Ok(None),
        Some(RequestValue::Raw(raw)) => serde_json::from_str::<Option<SessionTagValue>>(raw)
            .map_err(|source| S3ListingError::Field {
                field,
                source: Some(source),
            }),
        Some(value) => Ok(Some(session_tag_value(value))),
    }
}

fn merged_params(litellm_params: &Params, optional_params: &Params) -> Params {
    let mut merged = litellm_params.clone();
    merged.extend(optional_params.clone());
    merged
}

// {**litellm_params, **optional_params}: unlike B644's upload path the request side wins here
fn request_region(
    params: &Params,
    region_inputs: RegionInputs<'_>,
) -> Result<String, S3ListingError> {
    let s3_region = text_field(params, S3_REGION_PARAM)?;
    let aws_region = text_field(params, AWS_REGION_PARAM)?;
    let preference = match s3_region {
        Some(region) if !region.is_empty() => Some(region),
        Some(_) | None => aws_region,
    };
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
    let endpoint = match configured {
        Some(url) if !url.is_empty() => url,
        Some(_) | None => format!(
            "https://s3.{aws_region_name}.{}",
            get_aws_partition(Some(aws_region_name)).dns_suffix
        ),
    };
    Ok(endpoint.trim_end_matches('/').to_owned())
}

#[must_use]
pub fn s3_request_target(
    optional_params: &Params,
    litellm_params: &Params,
    region_inputs: RegionInputs<'_>,
) -> Result<S3RequestTarget, S3ListingError> {
    let params = merged_params(litellm_params, optional_params);
    let aws_region_name = request_region(&params, region_inputs)?;
    let endpoint_url = endpoint_url(&params, &aws_region_name)?;
    let auth = AuthParams {
        aws_access_key_id: secret_field(&params, "aws_access_key_id")?,
        aws_secret_access_key: secret_field(&params, "aws_secret_access_key")?,
        aws_session_token: secret_field(&params, "aws_session_token")?,
        aws_session_name: text_field(&params, "aws_session_name")?,
        aws_profile_name: text_field(&params, "aws_profile_name")?,
        aws_role_name: text_field(&params, "aws_role_name")?,
        aws_web_identity_token: secret_field(&params, "aws_web_identity_token")?,
        aws_sts_endpoint: text_field(&params, "aws_sts_endpoint")?,
        aws_external_id: text_field(&params, "aws_external_id")?,
        aws_session_tags: session_tags(&params, "aws_session_tags")?,
    };
    let s3 = AwsS3Settings {
        access_key_id: secret_field(&params, S3_ACCESS_KEY_ID_PARAM)?,
        secret_access_key: secret_field(&params, S3_SECRET_ACCESS_KEY_PARAM)?,
    };
    Ok(S3RequestTarget {
        endpoint_url,
        aws_region_name,
        auth,
        s3,
    })
}
