use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_context::cloud_storage::{Encoding, encode_object_name, split_bucket_name};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};

use super::decode::{DecodeLimits, decode_s3_listing};
use super::prefix::{
    listing_bucket_name, listing_query, output_location_still_unlisted, walked_listing_purpose,
};
use super::sign::{s3_credentials, sign_without_body};
use super::target::s3_request_target;
use super::{LIST_FILES_LOCATION_PARAM, LIST_FILES_PURPOSE_PARAM, S3ListingError};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params, RequestValue};
use crate::s3_access::S3AccessParams;
use crate::s3_access::managed::resolve_managed_s3_object;
use crate::s3_signing::SIGNED_HEADERS_PARAM;
use crate::signing::target::RegionInputs;

// Python's urlencode(..., quote_via=quote, safe="") keeps only letters, digits and -._~
const QUERY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3SignedRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
}

pub struct S3FilesRequest<'a> {
    pub optional_params: &'a Params,
    pub litellm_params: &'a mut Params,
    pub access: &'a S3AccessParams,
    pub environment: &'a BTreeMap<String, String>,
    pub region_inputs: RegionInputs<'a>,
    pub now: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3FileMethod {
    Content,
    Delete,
}

impl S3FileMethod {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Content => "GET",
            Self::Delete => "DELETE",
        }
    }
}

fn set_listing_value(params: &mut Params, name: &str, value: Option<&str>) {
    let stored = match value {
        Some(text) => RequestValue::Text(text.to_owned()),
        // LiteLLM writes None into litellm_params, and the next transform reads it as unset
        None => RequestValue::Raw("null".to_owned()),
    };
    params.insert(name.to_owned(), stored);
}

fn encode_query(query: &[(String, String)]) -> String {
    let mut encoded = String::new();
    for (index, (name, value)) in query.iter().enumerate() {
        if index > 0 {
            encoded.push('&');
        }
        encoded.push_str(&percent_encode(name.as_bytes(), QUERY_ENCODE_SET).to_string());
        encoded.push('=');
        encoded.push_str(&percent_encode(value.as_bytes(), QUERY_ENCODE_SET).to_string());
    }
    encoded
}

// B644's validate_environment shifts this entry out of litellm_params and sends it as request headers
fn store_signed_headers(params: &mut Params, headers: Vec<(String, String)>) {
    let entries: Params = headers
        .into_iter()
        .map(|(name, value)| (name, RequestValue::Text(value)))
        .collect();
    params.insert(
        SIGNED_HEADERS_PARAM.to_owned(),
        RequestValue::Mapping(entries),
    );
}

async fn signed_listing_request<F, Fut>(
    request: S3FilesRequest<'_>,
    purpose: Option<&str>,
    continuation_token: Option<&str>,
    resolve_credentials: F,
) -> Result<S3SignedRequest, S3ListingError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let bucket_name = listing_bucket_name(request.access, request.environment, purpose)?;
    let (bucket, configured_prefix) = split_bucket_name(&bucket_name)?;
    let target = s3_request_target(
        request.optional_params,
        request.litellm_params,
        request.region_inputs,
    )?;
    let url = format!("{}/{bucket}/", target.endpoint_url);
    let mut query = listing_query(&configured_prefix, purpose);
    if let Some(token) = continuation_token {
        query.push(("continuation-token".to_owned(), token.to_owned()));
    }
    let api_base = format!("{url}?{}", encode_query(&query));
    let credentials = s3_credentials(&target, resolve_credentials).await?;
    let signed_headers = sign_without_body("GET", &api_base, &target, &credentials, request.now)?;
    store_signed_headers(request.litellm_params, signed_headers);
    Ok(S3SignedRequest { url, query })
}

pub async fn list_files_request<F, Fut>(
    request: S3FilesRequest<'_>,
    purpose: Option<&str>,
    resolve_credentials: F,
) -> Result<S3SignedRequest, S3ListingError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    set_listing_value(request.litellm_params, LIST_FILES_PURPOSE_PARAM, purpose);
    set_listing_value(request.litellm_params, LIST_FILES_LOCATION_PARAM, purpose);
    signed_listing_request(request, purpose, None, resolve_credentials).await
}

pub async fn list_files_next_request<F, Fut>(
    request: S3FilesRequest<'_>,
    status: u16,
    body: &[u8],
    resolve_credentials: F,
) -> Result<Option<S3SignedRequest>, S3ListingError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    if status >= 400 {
        return Ok(None);
    }
    let listing = decode_s3_listing(body, &DecodeLimits::DEFAULT)?;
    if let Some(token) = listing.next_continuation_token {
        let purpose = walked_listing_purpose(request.litellm_params);
        let signed = signed_listing_request(
            request,
            purpose.as_deref(),
            Some(&token),
            resolve_credentials,
        );
        return signed.await.map(Some);
    }
    let still_unlisted = output_location_still_unlisted(
        request.access,
        request.environment,
        request.litellm_params,
    )?;
    if !still_unlisted {
        return Ok(None);
    }
    set_listing_value(
        request.litellm_params,
        LIST_FILES_LOCATION_PARAM,
        Some("batch_output"),
    );
    signed_listing_request(request, Some("batch_output"), None, resolve_credentials)
        .await
        .map(Some)
}

pub async fn object_file_request<F, Fut>(
    request: S3FilesRequest<'_>,
    method: S3FileMethod,
    file_id: Option<&str>,
    resolve_credentials: F,
) -> Result<S3SignedRequest, S3ListingError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let file_id = match method {
        S3FileMethod::Content => match file_id {
            Some(value) if !value.is_empty() => value,
            Some(_) | None => return Err(S3ListingError::FileIdRequired),
        },
        S3FileMethod::Delete => file_id.unwrap_or_default(),
    };
    let (bucket_name, object_key) =
        resolve_managed_s3_object(file_id, request.access, request.environment)?;
    let target = s3_request_target(
        request.optional_params,
        request.litellm_params,
        request.region_inputs,
    )?;
    let encoded = encode_object_name(&object_key, Encoding::S3);
    let url = format!("{}/{bucket_name}/{encoded}", target.endpoint_url);
    let credentials = s3_credentials(&target, resolve_credentials).await?;
    let signed_headers =
        sign_without_body(method.wire_name(), &url, &target, &credentials, request.now)?;
    store_signed_headers(request.litellm_params, signed_headers);
    Ok(S3SignedRequest {
        url,
        query: Vec::new(),
    })
}

// transform_retrieve_file_request and transform_retrieve_file_response both only raise
pub fn retrieve_file() -> Result<Infallible, S3ListingError> {
    tracing::debug!("BedrockFilesConfig does not support file retrieval");
    Err(S3ListingError::RetrievalUnsupported)
}
