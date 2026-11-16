use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use crate::a2a_chat::auth::EntraResolver;
use crate::ollama_chat::ids::IdSource;

pub(super) fn object(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

pub(super) fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn header_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) struct CountedIds {
    next: AtomicUsize,
}

impl CountedIds {
    pub(super) fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
        }
    }
}

impl IdSource for CountedIds {
    fn next_id(&self) -> String {
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        format!("id-{index}")
    }
}

pub(super) struct FakeEntra;

impl EntraResolver for FakeEntra {
    fn token(&self, params: &Object) -> Result<SecretString, CredentialsError> {
        let token = params
            .get("azure_ad_token")
            .and_then(Json::as_str)
            .unwrap_or("minted-token");
        Ok(SecretString::from(token.to_owned()))
    }

    fn auth_header(&self, _params: &Object) -> Result<BTreeMap<String, String>, CredentialsError> {
        Ok(header_map(&[("Authorization", "Bearer header-token")]))
    }
}

pub(super) struct RefusingEntra;

impl EntraResolver for RefusingEntra {
    fn token(&self, _params: &Object) -> Result<SecretString, CredentialsError> {
        Err(CredentialsError::AgentTokenUnresolved {
            help: "the test resolver mints no token",
        })
    }

    fn auth_header(&self, _params: &Object) -> Result<BTreeMap<String, String>, CredentialsError> {
        Err(CredentialsError::AgentTokenUnresolved {
            help: "the test resolver resolves no header",
        })
    }
}
