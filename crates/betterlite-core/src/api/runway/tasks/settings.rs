use betterlite_config::loader::EnvironmentLookup;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;

const DEFAULT_API_BASE: &str = "https://api.dev.runwayml.com";
const DEFAULT_API_VERSION: &str = "2024-11-06";
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

#[derive(Debug, Clone)]
pub struct RunwaySettings {
    pub api_base: String,
    pub api_secret: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub api_version: String,
    pub polling_timeout_seconds: u64,
}

impl RunwaySettings {
    pub fn load(lookup: &dyn EnvironmentLookup) -> Result<Self, RunwayTaskError> {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let polling_timeout_seconds = match present("RUNWAYML_POLLING_TIMEOUT") {
            Some(text) => betterlite_env::number("RUNWAYML_POLLING_TIMEOUT", &text)?
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            None => DEFAULT_TIMEOUT_SECONDS,
        };
        Ok(Self {
            api_base: present("RUNWAYML_API_BASE").unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            api_secret: present("RUNWAYML_API_SECRET").map(SecretString::from),
            api_key: present("RUNWAYML_API_KEY").map(SecretString::from),
            api_version: present("RUNWAYML_DEFAULT_API_VERSION")
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_owned()),
            polling_timeout_seconds,
        })
    }

    #[must_use]
    pub fn endpoint(&self, kind: TaskKind, api_base: Option<&str>) -> String {
        let base = api_base
            .filter(|explicit| !explicit.is_empty())
            .unwrap_or(&self.api_base)
            .trim_end_matches('/');
        format!("{base}/{}", kind.endpoint())
    }

    pub fn authenticate(
        &self,
        kind: TaskKind,
        headers: &HeaderMap,
        api_key: Option<&SecretString>,
    ) -> Result<HeaderMap, RunwayTaskError> {
        let credential = [api_key, self.api_secret.as_ref(), self.api_key.as_ref()]
            .into_iter()
            .flatten()
            .find(|secret| !secret.expose_secret().is_empty())
            .ok_or(RunwayTaskError::MissingCredentials)?;
        let mut bearer = HeaderValue::try_from(format!("Bearer {}", credential.expose_secret()))
            .map_err(RunwayTaskError::InvalidHeader)?;
        bearer.set_sensitive(true);
        let version =
            HeaderValue::try_from(&self.api_version).map_err(RunwayTaskError::InvalidHeader)?;
        let mut authenticated = headers.clone();
        authenticated.insert(AUTHORIZATION, bearer);
        authenticated.insert(HeaderName::from_static("x-runway-version"), version);
        if kind == TaskKind::Speech {
            authenticated.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        Ok(authenticated)
    }
}
