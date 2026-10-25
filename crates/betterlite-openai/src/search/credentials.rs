use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::{Position, Url};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CredentialError {
    #[error(
        "refusing to send the server-configured {credential} to the caller-supplied api_base '{api_base}'; pass an explicit api_key when overriding api_base for this search provider"
    )]
    UntrustedBase {
        credential: &'static str,
        api_base: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ServerCredential {
    pub key_vars: &'static [&'static str],
    pub base_var: Option<&'static str>,
    pub default_base: Option<&'static str>,
}

impl ServerCredential {
    pub fn resolve(
        &self,
        caller_key: Option<SecretString>,
        caller_base: Option<&str>,
        lookup: &dyn EnvironmentLookup,
    ) -> Result<Option<SecretString>, CredentialError> {
        if let Some(key) = caller_key.filter(|key| !key.expose_secret().is_empty()) {
            return Ok(Some(key));
        }
        let Some((credential, _)) = self.key_vars.split_first() else {
            return Ok(None);
        };
        let Some(server_key) = self
            .key_vars
            .iter()
            .find_map(|name| lookup.lookup(name).filter(|value| !value.is_empty()))
            .map(SecretString::from)
        else {
            return Ok(None);
        };
        match caller_base.filter(|base| !base.is_empty()) {
            Some(api_base) if !self.is_trusted(api_base, lookup) => {
                Err(CredentialError::UntrustedBase {
                    credential,
                    api_base: api_base.to_owned(),
                })
            }
            Some(_) | None => Ok(Some(server_key)),
        }
    }

    fn is_trusted(&self, api_base: &str, lookup: &dyn EnvironmentLookup) -> bool {
        let Some(candidate) = netloc(api_base).filter(|host| !host.is_empty()) else {
            return false;
        };
        let override_base = self.base_var.and_then(|name| lookup.lookup(name));
        [self.default_base, override_base.as_deref()]
            .into_iter()
            .flatten()
            .filter(|base| !base.is_empty())
            .any(|base| netloc(base).as_ref() == Some(&candidate))
    }
}

fn netloc(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    Some(parsed[Position::BeforeUsername..Position::AfterPort].to_lowercase())
}
