use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use http::Method;
use serde::Deserialize;

use crate::commands::keys::api::{self, Reply};
use crate::commands::keys::output;
use crate::commands::{Payload, Request};

use super::{EncryptionError, EncryptionPorts, MigrateEncryption};

const MIGRATE_PATH: &str = "/credentials/migrate-encryption";
const CHECK_PATH: &str = "/credentials/migrate-encryption/check";

#[derive(Debug, Deserialize)]
struct MigrateReply {
    #[serde(default)]
    report: Option<BTreeMap<String, JsonValue>>,
}

#[derive(Debug, Deserialize)]
struct MigrationReport {
    #[serde(default)]
    residual_legacy: Option<i64>,
}

pub(super) fn migrate_encryption(
    options: &MigrateEncryption,
    ports: EncryptionPorts<'_>,
) -> Result<(), EncryptionError> {
    let EncryptionPorts { client, stdout, .. } = ports;
    let (method, path, payload) = if options.check {
        (Method::GET, CHECK_PATH, Payload::Empty)
    } else {
        (Method::POST, MIGRATE_PATH, Payload::Json(b"{}".to_vec()))
    };
    let mut url = client.resolve(path)?;
    if options.dry_run && !options.check {
        url.query_pairs_mut().append_pair("dry_run", "true");
    }
    let response = client.send(Request {
        method: method.clone(),
        url: url.clone(),
        headers: Vec::new(),
        payload,
    })?;
    let reply = Reply {
        method,
        url: url.to_string(),
        status: response.status,
        body: response.body,
    };
    let value = api::interpret(&reply, None)?;
    output::write_json(&value, output::JsonText::Pretty, stdout).map_err(EncryptionError::Json)?;
    if !value.is_object() {
        return Err(EncryptionError::NotAnObject { url: reply.url });
    }
    match residual_legacy(value)? {
        Some(residual) if residual > 0 => {
            let _wrote = writeln!(stdout, "Residual legacy values remaining: {residual}");
        }
        Some(0) => {
            let _wrote = writeln!(stdout, "No legacy values remaining (residual_legacy == 0).");
        }
        Some(_) | None => {}
    }
    Ok(())
}

fn residual_legacy(value: JsonValue) -> Result<Option<i64>, EncryptionError> {
    let reply: MigrateReply =
        serde_path_to_error::deserialize(value).map_err(|error| EncryptionError::ReplyField {
            field: error.path().to_string(),
            source: error.into_inner(),
        })?;
    let report_object = JsonValue::Object(reply.report.unwrap_or_default().into_iter().collect());
    let report: MigrationReport =
        serde_path_to_error::deserialize(report_object).map_err(|error| {
            EncryptionError::ReplyField {
                field: format!("report.{}", error.path()),
                source: error.into_inner(),
            }
        })?;
    Ok(report.residual_legacy)
}
