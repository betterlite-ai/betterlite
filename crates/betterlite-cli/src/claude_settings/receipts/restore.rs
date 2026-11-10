use betterlite_config::sections::settings::JsonValue;

use super::super::{
    ANTHROPIC_BASE_URL_KEY, BASE_URL_PATH, CREDENTIAL_PATHS, ENV_KEY, OWNED_PATHS, SettingsDocument,
};
use super::lookup::{env_is_empty, lookup, with};
use super::record::{ConfigureReceipt, owns};
use super::unconfigure::WithheldCredential;
use super::value::{ABSENT, OwnedValue, fingerprint};

pub(super) struct Restored {
    pub(super) settings: SettingsDocument,
    pub(super) withheld: Vec<WithheldCredential>,
}

pub(super) fn owned_and_kept(
    current: &SettingsDocument,
    receipt: &ConfigureReceipt,
) -> (Vec<String>, Vec<String>) {
    let mut ours = Vec::new();
    let mut kept = Vec::new();
    for path in written_paths(receipt) {
        let still_ours = owns(current, &path, receipt);
        let user_kept = !still_ours && lookup(current, &path).present;
        if user_kept {
            kept.push(path.clone());
        }
        if still_ours {
            ours.push(path);
        }
    }
    (ours, kept)
}

pub(super) fn restored_document(
    current: &SettingsDocument,
    receipt: &ConfigureReceipt,
    ours: &[String],
) -> Restored {
    let mut put_back = current.clone();
    for path in ours {
        let previous = receipt.previous.get(path).cloned().unwrap_or(ABSENT);
        with(&mut put_back, path, &previous);
    }
    let url_after = lookup(&put_back, BASE_URL_PATH);
    let mut withheld = Vec::new();
    for path in CREDENTIAL_PATHS {
        let previous = receipt.previous.get(path).cloned().unwrap_or(ABSENT);
        if !previous.present || !ours.iter().any(|ours_path| ours_path == path) {
            continue;
        }
        let endpoint = receipt.endpoints.get(path).cloned().unwrap_or(ABSENT);
        if endpoint != url_after {
            withheld.push(WithheldCredential {
                key: path.to_owned(),
                endpoint: endpoint_text(&endpoint),
            });
        }
    }
    let mut settings = put_back;
    for item in &withheld {
        with(&mut settings, &item.key, &ABSENT);
    }
    if env_is_empty(&settings) && !receipt.env_was_object {
        let env = OwnedValue {
            present: receipt.env_present,
            value: JsonValue::Null,
        };
        with(&mut settings, ENV_KEY, &env);
    }
    Restored { settings, withheld }
}

pub(super) fn withheld_receipt(
    receipt: &ConfigureReceipt,
    withheld: &[WithheldCredential],
) -> Option<ConfigureReceipt> {
    if withheld.is_empty() {
        return None;
    }
    let absent = fingerprint(&ABSENT);
    Some(ConfigureReceipt {
        file_existed: receipt.file_existed,
        env_present: receipt.env_present,
        env_was_object: receipt.env_was_object,
        previous: receipt.previous.clone(),
        written: withheld
            .iter()
            .map(|item| (item.key.clone(), absent.clone()))
            .collect(),
        endpoints: receipt.endpoints.clone(),
    })
}

fn written_paths(receipt: &ConfigureReceipt) -> Vec<String> {
    let mut paths: Vec<String> = OWNED_PATHS
        .iter()
        .filter(|path| receipt.written.contains_key(**path))
        .map(|path| (*path).to_owned())
        .collect();
    for path in receipt.written.keys() {
        if !OWNED_PATHS.contains(&path.as_str()) {
            paths.push(path.clone());
        }
    }
    paths
}

fn endpoint_text(endpoint: &OwnedValue) -> String {
    if !endpoint.present {
        return format!("no {ANTHROPIC_BASE_URL_KEY} (Anthropic's default endpoint)");
    }
    match &endpoint.value {
        JsonValue::String(text) => text.clone(),
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => other.to_string(),
    }
}
