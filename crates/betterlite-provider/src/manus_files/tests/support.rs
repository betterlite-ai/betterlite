use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::files::records::{CreatePurpose, FileCreate, FileUpload};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::file_contracts::config::{FileParams, FileResponse, Headers};
use crate::manus_files::ManusFilesConfig;

pub(super) const NOW: i64 = 1_700_000_000;

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC network"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the fixture has no Azure token provider"),
        })
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn config(
    globals: GlobalCredentials,
    environment: BTreeMap<String, String>,
    now: i64,
) -> ManusFilesConfig {
    ManusFilesConfig {
        globals,
        secrets: secrets(),
        environment,
        clock: Box::new(move || now),
    }
}

pub(super) fn plain_config(now: i64) -> ManusFilesConfig {
    config(GlobalCredentials::default(), BTreeMap::new(), now)
}

pub(super) fn globals(api_base: Option<&str>, api_key: Option<&str>) -> GlobalCredentials {
    GlobalCredentials {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
        xai_key: None,
        inception_key: None,
        lemonade_key: None,
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn key(value: &str) -> SecretString {
    SecretString::from(String::from(value))
}

pub(super) fn params(wire: &str) -> FileParams {
    serde_json::from_str(wire).expect("the fixture is a JSON object")
}

pub(super) fn upload(filename: Option<&str>, data: &[u8]) -> FileUpload {
    FileUpload {
        filename: filename.map(str::to_owned),
        content_type: None,
        data: data.to_vec(),
    }
}

pub(super) fn create_file(filename: Option<&str>, data: &[u8]) -> FileCreate {
    FileCreate {
        file: upload(filename, data),
        purpose: CreatePurpose::Assistants,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

pub(super) fn response(body: &str) -> FileResponse {
    FileResponse {
        status_code: 200,
        headers: Headers::new(),
        body: body.as_bytes().to_vec(),
        url: None,
    }
}
