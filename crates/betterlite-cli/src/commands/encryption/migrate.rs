use std::io::Write;

use betterlite_config::sections::settings::JsonValue;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

use crate::commands::keys::api::{self, Call};
use crate::commands::keys::output;

use super::{EncryptionError, EncryptionPorts, MigrateEncryption};

const MIGRATE_PATH: &str = "/credentials/migrate-encryption";
const CHECK_PATH: &str = "/credentials/migrate-encryption/check";

pub(super) fn migrate_encryption(
    options: &MigrateEncryption,
    ports: EncryptionPorts<'_>,
) -> Result<(), EncryptionError> {
    let EncryptionPorts {
        context,
        client,
        stdout,
        ..
    } = ports;
    let (method, path, query, body) = request_parts(options);
    let mut headers = HeaderMap::new();
    if body.is_some() {
        let _previous = headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    let reply = api::call(
        client,
        Call {
            method,
            base_url: context.base_url.trim_end_matches('/'),
            path,
            query: &query,
            headers,
            body,
            api_key: context.api_key.as_ref(),
        },
    )?;
    let value = api::interpret(&reply, None)?;
    output::write_json(&value, output::JsonText::Pretty, stdout).map_err(EncryptionError::Json)?;
    print_residual(&value, &reply.url, stdout)?;
    Ok(())
}

fn request_parts(
    options: &MigrateEncryption,
) -> (Method, &'static str, Vec<(String, String)>, Option<Vec<u8>>) {
    if options.check {
        return (Method::GET, CHECK_PATH, Vec::new(), None);
    }
    let query = if options.dry_run {
        vec![(String::from("dry_run"), String::from("true"))]
    } else {
        Vec::new()
    };
    (Method::POST, MIGRATE_PATH, query, Some(b"{}".to_vec()))
}

fn print_residual(
    value: &JsonValue,
    url: &str,
    stdout: &mut dyn Write,
) -> Result<(), EncryptionError> {
    let Some(object) = value.as_object() else {
        return Err(EncryptionError::NotAnObject {
            url: url.to_owned(),
        });
    };
    let Some(report_value) = object.get("report") else {
        return Ok(());
    };
    if report_value.is_null() {
        return Ok(());
    }
    let Some(report) = report_value.as_object() else {
        return Err(EncryptionError::ReportNotObject);
    };
    let Some(residual_value) = report.get("residual_legacy") else {
        return Ok(());
    };
    if residual_value.is_null() {
        return Ok(());
    }
    let Some(residual) = residual_value.as_i64() else {
        return Err(EncryptionError::ResidualNotInteger);
    };
    if residual > 0 {
        let _wrote = writeln!(stdout, "Residual legacy values remaining: {residual}");
    } else if residual == 0 {
        let _wrote = writeln!(stdout, "No legacy values remaining (residual_legacy == 0).");
    }
    Ok(())
}
