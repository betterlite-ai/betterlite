use betterlite_config::loader::EnvironmentLookup;
use betterlite_context::aws_partition::get_aws_partition;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::s3_transport::error::S3TransportError;

mod tests;

const DEFAULT_REGION: &str = "us-west-2";
const REGION_ENVIRONMENT: [&str; 2] = ["AWS_REGION", "AWS_REGION_NAME"];
const OBJECT_KEY_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Endpoint {
    pub bucket: String,
    pub region: String,
    pub endpoint_url: Option<String>,
    pub virtual_hosted_style: bool,
}

impl S3Endpoint {
    #[must_use]
    pub fn object_url(&self, object_key: &str) -> String {
        let encoded_key = utf8_percent_encode(object_key, OBJECT_KEY_SAFE);
        match &self.endpoint_url {
            Some(endpoint) if self.virtual_hosted_style => {
                let host = endpoint.replace("https://", "").replace("http://", "");
                let protocol = if endpoint.starts_with("https://") {
                    "https://"
                } else {
                    "http://"
                };
                format!("{protocol}{}.{host}/{encoded_key}", self.bucket)
            }
            Some(endpoint) => format!("{endpoint}/{}/{encoded_key}", self.bucket),
            None => {
                let dns_suffix = get_aws_partition(Some(&self.region)).dns_suffix;
                let bucket = &self.bucket;
                let region = &self.region;
                format!("https://{bucket}.s3.{region}.{dns_suffix}/{encoded_key}")
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EndpointInputs<'text> {
    pub bucket: Option<&'text str>,
    pub region: Option<&'text str>,
    pub endpoint_url: Option<&'text str>,
    pub virtual_hosted_style: bool,
}

pub fn resolve_endpoint(
    inputs: EndpointInputs<'_>,
    environment: &dyn EnvironmentLookup,
) -> Result<S3Endpoint, S3TransportError> {
    let bucket = inputs
        .bucket
        .filter(|bucket| !bucket.is_empty())
        .ok_or(S3TransportError::MissingBucket)?;
    Ok(S3Endpoint {
        bucket: bucket.to_owned(),
        region: resolve_region(inputs.region, environment)?,
        endpoint_url: inputs.endpoint_url.map(str::to_owned),
        virtual_hosted_style: inputs.virtual_hosted_style,
    })
}

pub fn resolve_region(
    configured: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> Result<String, S3TransportError> {
    if let Some(region) = configured {
        return validated_region(region);
    }
    let from_environment = REGION_ENVIRONMENT
        .iter()
        .find_map(|name| environment.lookup(name).filter(|region| !region.is_empty()));
    validated_region(from_environment.as_deref().unwrap_or(DEFAULT_REGION))
}

fn validated_region(region: &str) -> Result<String, S3TransportError> {
    // LiteLLM's _VALID_AWS_REGION_PATTERN is \A[a-z0-9-]+\Z
    let valid = !region.is_empty()
        && region
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(region.to_owned())
    } else {
        Err(S3TransportError::Region {
            region: region.to_owned(),
        })
    }
}
