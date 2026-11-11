use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use crate::aws_settings::sources::Credentials;
use crate::invoke_primitives::config::Params;
use crate::request_policy::{Params as MetadataParams, RequestValue};

pub(crate) fn params(json_text: &str) -> Params {
    serde_json::from_str(json_text).unwrap()
}

pub(crate) fn messages(json_text: &str) -> Vec<Message> {
    serde_json::from_str(json_text).unwrap()
}

pub(crate) fn request_data(json_text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(json_text).unwrap()
}

pub(crate) fn clock() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

pub(crate) fn epoch(seconds: i64) -> SystemTime {
    let bounded = u64::try_from(seconds).unwrap();
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .unwrap()
}

pub(crate) fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

pub(crate) fn empty_factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Unreachable,
    }
}

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(crate) fn credentials(access: &str, secret: &str) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access.to_owned()),
        secret_access_key: SecretString::from(secret.to_owned()),
        session_token: None,
    }
}

pub(crate) fn text(value: &str) -> RequestValue {
    RequestValue::Text(value.to_owned())
}

pub(crate) fn mapping(pairs: &[(&str, RequestValue)]) -> RequestValue {
    RequestValue::Mapping(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

pub(crate) fn metadata(pairs: &[(&str, RequestValue)]) -> MetadataParams {
    IndexMap::from([("metadata".to_owned(), mapping(pairs))])
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
