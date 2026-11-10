#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use secrecy::{ExposeSecret as _, SecretString};

use crate::endpoint_target::target::{AuthHeader, EndpointTarget};
use crate::model_urls::ApiVersion;

pub(super) const PROJECT: &str = "proj";
pub(super) const LOCATION: &str = "us-central1";
pub(super) const MODEL: &str = "gemini-2.5-flash";
pub(super) const NATIVE_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent";

pub(super) fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(super) fn exposed(value: &SecretString) -> &str {
    value.expose_secret()
}

pub(super) fn token_of(target: &EndpointTarget) -> Option<&str> {
    match &target.auth {
        Some(AuthHeader::Token(token)) => Some(exposed(token)),
        None | Some(AuthHeader::Headers(_)) => None,
    }
}

pub(super) fn header_of<'target>(
    target: &'target EndpointTarget,
    name: &str,
) -> Option<&'target str> {
    match &target.auth {
        Some(AuthHeader::Headers(headers)) => headers.get(name).map(exposed),
        Some(AuthHeader::Token(_)) | None => None,
    }
}

pub(super) fn version_of(text: &str) -> ApiVersion {
    if text == "v1beta1" {
        ApiVersion::V1Beta1
    } else {
        ApiVersion::V1
    }
}

pub(super) fn expected_headers(extras: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    for (name, value) in extras {
        headers.insert((*name).to_owned(), (*value).to_owned());
    }
    headers
}
