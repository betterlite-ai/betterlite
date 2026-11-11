use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::agentcore_request::message::AgentCoreMessage;
use crate::aws_settings::sources::Credentials;
use crate::model_routes::ParamMap;

pub(super) const SESSION_UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
pub(super) const NOW_SECONDS: u64 = 1_767_225_600;

pub(super) fn params(source: &str) -> ParamMap {
    serde_json::from_str(source).unwrap()
}

pub(super) fn messages(source: &str) -> Vec<AgentCoreMessage> {
    serde_json::from_str(source).unwrap()
}

pub(super) fn request_data(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).unwrap()
}

pub(super) fn headers(pairs: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.map(str::to_owned)))
        .collect()
}

pub(super) trait HeaderText {
    fn text(&self) -> Option<&str>;
}

impl HeaderText for Option<String> {
    fn text(&self) -> Option<&str> {
        self.as_deref()
    }
}

impl HeaderText for String {
    fn text(&self) -> Option<&str> {
        Some(self.as_str())
    }
}

pub(super) fn header<'headers, Value>(
    headers: &'headers [(String, Value)],
    name: &str,
) -> Option<&'headers str>
where
    Value: HeaderText,
{
    headers
        .iter()
        .find(|(found, _)| found == name)
        .and_then(|(_, value)| value.text())
}

pub(super) fn generated_uuid() -> Uuid {
    Uuid::parse_str(SESSION_UUID).unwrap()
}

pub(super) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap()
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn fixture_credentials(access: &str) -> Credentials {
    Credentials {
        access_key_id: secret(access),
        secret_access_key: secret("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"),
        session_token: None,
    }
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
