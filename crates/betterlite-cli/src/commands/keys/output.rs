use std::io::Write;

use betterlite_config::sections::settings::JsonValue;

use crate::commands::tables::write_table;

use super::KeysError;
use super::dates;
use super::wire::SourceKey;

#[derive(Clone, Copy)]
pub(in crate::commands) enum JsonText {
    Pretty,
    Generated,
}

pub(in crate::commands) fn write_json(
    value: &JsonValue,
    text: JsonText,
    stdout: &mut dyn Write,
) -> Result<(), serde_json::Error> {
    let (prefix, rendered) = match text {
        JsonText::Pretty => ("", serde_json::to_string_pretty(value)?),
        JsonText::Generated => ("Generated key: ", serde_json::to_string(value)?),
    };
    let _wrote = writeln!(stdout, "{prefix}{rendered}");
    Ok(())
}

pub(super) fn print_http_error(
    status: u16,
    body: &[u8],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), serde_json::Error> {
    let _status_line = writeln!(stderr, "Error: HTTP {status}");
    match serde_json::from_slice::<JsonValue>(body) {
        Ok(value) => write_json(&value, JsonText::Pretty, stdout),
        Err(_source) => {
            let _wrote = stderr.write_all(body);
            if !body.ends_with(b"\n") {
                let _newline = stderr.write_all(b"\n");
            }
            Ok(())
        }
    }
}

pub(super) fn write_dry_run_table(
    keys: &[SourceKey],
    stdout: &mut dyn Write,
) -> Result<(), KeysError> {
    let _marker = writeln!(stdout, "\n--- DRY RUN MODE ---");
    let mut rows = Vec::new();
    for key in keys {
        let created = match key.created_at.as_deref() {
            None => String::new(),
            Some(text) => dates::parse_created_at(text)
                .map_err(|source| KeysError::CreatedAt {
                    value: text.to_owned(),
                    source,
                })?
                .format("%Y-%m-%d %H:%M")
                .to_string(),
        };
        rows.push(vec![
            key.key_alias.clone().unwrap_or_default(),
            key.user_id.clone().unwrap_or_default(),
            created,
        ]);
    }
    write_table(
        "Keys that would be imported",
        &["Key Alias", "User ID", "Created"],
        rows,
        stdout,
    );
    Ok(())
}
