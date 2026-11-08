pub mod idp_documents;
pub mod incoming_token;
pub mod jwks_document;
pub mod sign_request;
pub mod signer_settings;
pub mod signing_key;
pub mod upstream_headers;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_config::loader::EnvironmentLookup;
use reqwest::Client;
use thiserror::Error;

use crate::mcp_jwt_signer::idp_documents::IdpClient;
use crate::mcp_jwt_signer::signer_settings::{SettingsError, SignerOptions, SignerSettings};
use crate::mcp_jwt_signer::signing_key::{SigningKey, SigningKeyError};

pub struct JwtSigner {
    pub(crate) settings: SignerSettings,
    pub(crate) key: SigningKey,
    pub(crate) idp: IdpClient,
}

#[derive(Debug, Error)]
pub enum JwtSignerError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    SigningKey(#[from] SigningKeyError),
}

impl JwtSigner {
    pub fn new(
        options: &SignerOptions,
        env: &dyn EnvironmentLookup,
        client: Client,
    ) -> Result<Self, JwtSignerError> {
        let settings = SignerSettings::resolve(options, env)?;
        let key = SigningKey::load(env)?;
        Ok(Self {
            idp: IdpClient {
                client,
                timeout: settings.timeout,
                jwks_cache: Mutex::new(BTreeMap::new()),
                discovery_cache: Mutex::new(BTreeMap::new()),
            },
            settings,
            key,
        })
    }
}
