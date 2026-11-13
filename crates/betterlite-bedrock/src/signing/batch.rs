use super::error::SigningError;
use super::headers::{merge_computed, prepare, without_computed};
use super::request::AwsSignedRequest;
use super::sigv4::{Signable, sign_request};
use super::target::{RegionInputs, auth_settings, resolve_region};
use crate::aws_settings::pop_aws_auth_params;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;
#[derive(Debug, Clone, Copy)]
pub enum BatchData<'source> {
    Json(&'source IndexMap<String, Json>),
    Text(&'source str),
}
#[derive(Debug, Clone, Copy)]
pub struct BatchRequest<'source> {
    pub service_name: &'source str,
    pub data: BatchData<'source>,
    pub endpoint: &'source str,
    pub optional_params: &'source BTreeMap<String, Box<RawValue>>,
    pub method: &'source str,
    pub region_inputs: RegionInputs<'source>,
    pub now: SystemTime,
}
pub async fn sign_batch_request<F, Fut>(
    request: BatchRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let region = resolve_region(request.optional_params, Some(""), request.region_inputs)?;
    let mut copy = request.optional_params.clone();
    let auth = pop_aws_auth_params(&mut copy).map_err(SigningError::Auth)?;
    let settings = auth_settings(auth, &region);
    let credentials = resolve_credentials(&settings, &region)
        .await
        .map_err(SigningError::Credential)?;
    let method = request.method.to_ascii_uppercase();
    let (body, initial_headers) = if method == "GET" {
        (String::new(), Vec::new())
    } else {
        let text = match request.data {
            BatchData::Json(object) => dumps_object(object),
            BatchData::Text(text) => text.to_owned(),
        };
        (
            text,
            vec![("Content-Type".to_owned(), "application/json".to_owned())],
        )
    };
    let supplied = without_computed(initial_headers, credentials.session_token.is_some());
    let computed = sign_request(&Signable {
        method: &method,
        url: request.endpoint,
        headers: &supplied,
        body: body.as_bytes(),
        service: request.service_name,
        region: &region,
        credentials: &credentials,
        now: request.now,
    })?;
    let merged = merge_computed(supplied, computed);
    Ok(AwsSignedRequest {
        method: method.clone(),
        url: request.endpoint.to_owned(),
        headers: prepare(merged, body.chars().count(), &method),
        body: body.into_bytes(),
    })
}
