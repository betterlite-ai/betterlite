use std::collections::BTreeMap;
use std::path::Path;

use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

use super::super::{BASE_URL_PATH, CREDENTIAL_PATHS, ENV_KEY, OWNED_PATHS, SettingsDocument};
use super::error::ReceiptsError;
use super::lookup::lookup;
use super::value::{OwnedValue, fingerprint};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the receipt records LiteLLM's three independent file and env flags"
)]
pub struct ConfigureReceipt {
    pub file_existed: bool,
    pub env_present: bool,
    pub env_was_object: bool,
    pub previous: BTreeMap<String, OwnedValue>,
    pub written: BTreeMap<String, String>,
    pub endpoints: BTreeMap<String, OwnedValue>,
}

struct Claim {
    previous: OwnedValue,
    written: Option<String>,
    endpoint: Option<OwnedValue>,
}

pub(super) fn owns(settings: &SettingsDocument, path: &str, receipt: &ConfigureReceipt) -> bool {
    let Some(expected) = receipt.written.get(path) else {
        return false;
    };
    *expected == fingerprint(&lookup(settings, path))
}

pub(super) fn build_receipt(
    current: &SettingsDocument,
    merged: &SettingsDocument,
    earlier: Option<&ConfigureReceipt>,
    file_existed: bool,
) -> ConfigureReceipt {
    let url_now = lookup(current, BASE_URL_PATH);
    let mut previous = BTreeMap::new();
    let mut written = BTreeMap::new();
    let mut endpoints = BTreeMap::new();
    for path in OWNED_PATHS {
        let recorded = claim(path, current, merged, earlier, &url_now);
        previous.insert(path.to_owned(), recorded.previous);
        if let Some(value) = recorded.written {
            written.insert(path.to_owned(), value);
        }
        if let Some(endpoint) = recorded.endpoint {
            endpoints.insert(path.to_owned(), endpoint);
        }
    }
    let env_now = current.contains_key(ENV_KEY);
    let env_object_now = matches!(current.get(ENV_KEY), Some(JsonValue::Object(_)));
    ConfigureReceipt {
        file_existed: earlier.map_or(file_existed, |prior| prior.file_existed),
        env_present: earlier.map_or(env_now, |prior| prior.env_present),
        env_was_object: earlier.map_or(env_object_now, |prior| prior.env_was_object),
        previous,
        written,
        endpoints,
    }
}

fn claim(
    path: &str,
    current: &SettingsDocument,
    merged: &SettingsDocument,
    earlier: Option<&ConfigureReceipt>,
    url_now: &OwnedValue,
) -> Claim {
    let before = lookup(current, path);
    let after = lookup(merged, path);
    let carried = earlier.filter(|prior| owns(current, path, prior));
    let previous = carried
        .and_then(|prior| prior.previous.get(path).cloned())
        .unwrap_or_else(|| before.clone());
    let written = if before == after {
        earlier.and_then(|prior| prior.written.get(path).cloned())
    } else {
        Some(fingerprint(&after))
    };
    let endpoint = CREDENTIAL_PATHS.contains(&path).then(|| {
        carried
            .and_then(|prior| prior.endpoints.get(path).cloned())
            .unwrap_or_else(|| url_now.clone())
    });
    Claim {
        previous,
        written,
        endpoint,
    }
}

pub(super) fn receipt_document(
    receipt: &ConfigureReceipt,
    state_path: &Path,
) -> Result<JsonValue, ReceiptsError> {
    serde_json::to_value(receipt).map_err(|source| ReceiptsError::EncodeReceipt {
        path: state_path.to_owned(),
        source,
    })
}
