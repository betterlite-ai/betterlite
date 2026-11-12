use std::cmp::Ordering;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::{ExposeSecret as _, SecretString};

// LiteLLM identifies cached credentials by the serialized credential value and the requested project, never by model
#[derive(Debug, Clone)]
pub struct CredentialKey {
    pub credential: Option<SecretString>,
    pub project: Option<String>,
}

impl CredentialKey {
    #[must_use]
    pub fn new(credentials: Option<&ConfiguredCredentials>, project: Option<&str>) -> Self {
        Self {
            credential: credentials
                .map(|configured| SecretString::from(serialized_credential(configured))),
            project: project.map(str::to_owned),
        }
    }
}

// LiteLLM keys a dict credential by json.dumps and every other credential by the text it already holds
fn serialized_credential(credentials: &ConfiguredCredentials) -> String {
    match credentials {
        ConfiguredCredentials::Source(source) => source.document(),
        ConfiguredCredentials::Text(text) => text.expose_secret().to_owned(),
    }
}

fn credential_order(left: Option<&SecretString>, right: Option<&SecretString>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(left_secret), Some(right_secret)) => left_secret
            .expose_secret()
            .cmp(right_secret.expose_secret()),
    }
}

impl PartialEq for CredentialKey {
    fn eq(&self, other: &Self) -> bool {
        self.project == other.project
            && credential_order(self.credential.as_ref(), other.credential.as_ref())
                == Ordering::Equal
    }
}

impl Eq for CredentialKey {}

impl PartialOrd for CredentialKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CredentialKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.project
            .cmp(&other.project)
            .then_with(|| credential_order(self.credential.as_ref(), other.credential.as_ref()))
    }
}
