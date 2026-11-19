use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_azure::search::minter::{EntraTokenMinter, MintError};
use betterlite_config::sections::azure_search::{
    CONNECTION_ID_ENV, MODEL_ENV, PROJECT_ENDPOINT_ENV, Settings, TOKEN_ENV,
};
use secrecy::SecretString;

use super::scenarios::MinterSpec;

pub(crate) fn settings(environment: &BTreeMap<String, String>) -> Settings {
    Settings {
        project_endpoint: environment.get(PROJECT_ENDPOINT_ENV).cloned(),
        model: environment.get(MODEL_ENV).cloned(),
        connection_id: environment.get(CONNECTION_ID_ENV).cloned(),
        token: environment.get(TOKEN_ENV).cloned().map(SecretString::from),
    }
}

pub(crate) struct FixtureMinter {
    spec: MinterSpec,
    pub(crate) calls: AtomicUsize,
}

impl FixtureMinter {
    pub(crate) const fn new(spec: MinterSpec) -> Self {
        Self {
            spec,
            calls: AtomicUsize::new(0),
        }
    }
}

impl EntraTokenMinter for FixtureMinter {
    fn mint(&self) -> Result<SecretString, MintError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let Some(detail) = &self.spec.error else {
            return Ok(SecretString::from(self.spec.token.clone()));
        };
        Err(MintError::Detail {
            detail: detail.clone(),
        })
    }
}
