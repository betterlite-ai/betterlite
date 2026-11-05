use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;
use crate::api::runway::tasks::settings::RunwaySettings;

impl RunwaySettings {
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
