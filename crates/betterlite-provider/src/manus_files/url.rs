use std::collections::BTreeMap;

use betterlite_http::url_path::encode_segment;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::ExposeSecret as _;

use crate::api::file_contracts::config::FileParams;

use super::credentials::secret_text;
use super::error::ManusFilesError;
use super::params::text;
use super::{FILE_ID_FIELD, MANUS_API_BASE, MANUS_API_BASE_ENV};

const V1_SUFFIX: &str = "/v1";

pub(super) fn files_url(
    explicit: Option<&str>,
    globals: &GlobalCredentials,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<String, ManusFilesError> {
    let chosen = explicit
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            globals
                .api_base
                .as_ref()
                .filter(|base| !base.is_empty())
                .cloned()
        });
    let base = match chosen {
        Some(base) => base,
        None => secret_text(secrets, environment, MANUS_API_BASE_ENV)?.map_or_else(
            || String::from(MANUS_API_BASE),
            |text| text.expose_secret().to_owned(),
        ),
    };
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(V1_SUFFIX) {
        Ok(format!("{trimmed}/files"))
    } else {
        Ok(format!("{trimmed}{V1_SUFFIX}/files"))
    }
}

pub(super) fn litellm_files_url(
    litellm_params: &FileParams,
    globals: &GlobalCredentials,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<String, ManusFilesError> {
    files_url(
        text(litellm_params, "api_base")?,
        globals,
        secrets,
        environment,
    )
}

pub(super) fn encoded_file_id(file_id: &str) -> Result<String, ManusFilesError> {
    encode_segment(file_id, FILE_ID_FIELD).map_err(|source| ManusFilesError::FileId {
        file_id: file_id.to_owned(),
        source,
    })
}
