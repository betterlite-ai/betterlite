use std::io::Write;

use http::Method;

use crate::commands::tables::{OutputFormat, write_table};

use super::client::KeysClient;
use super::output;
use super::wire::KeyListPage;
use super::{KeysError, KeysPorts, ListKeys};

const LIST_PATH: &str = "/key/list";

pub(super) fn list_keys(options: &ListKeys, ports: KeysPorts<'_>) -> Result<(), KeysError> {
    let KeysPorts {
        context,
        client,
        redactor,
        stdout,
        stderr: _stderr,
    } = ports;
    let query = list_query(options);
    let api = KeysClient {
        transport: client,
        base_url: context.base_url.trim_end_matches('/'),
        api_key: context.api_key.as_ref(),
        redactor,
    };
    let reply = api.send(Method::GET, LIST_PATH, &query, None)?;
    let value = reply.value;
    if !value.is_object() {
        return Err(KeysError::NotAnObject { url: reply.url });
    }
    if options.format == OutputFormat::Json {
        return output::write_json(&value, output::JsonText::Pretty, stdout)
            .map_err(KeysError::Json);
    }
    let page: KeyListPage =
        serde_path_to_error::deserialize(value).map_err(|error| KeysError::Field {
            field: error.path().to_string(),
            source: error.into_inner(),
        })?;
    write_keys_table(page, stdout);
    Ok(())
}

fn write_keys_table(page: KeyListPage, stdout: &mut dyn Write) {
    let count = page.keys.len();
    let total = page.total_count;
    let _showing = writeln!(stdout, "Showing {count} keys out of {total}");
    let rows = page
        .keys
        .into_iter()
        .map(|key| {
            vec![
                key.token.unwrap_or_default(),
                key.key_alias.unwrap_or_default(),
                key.user_id.unwrap_or_default(),
                key.team_id.unwrap_or_default(),
                key.spend.map(|spend| spend.to_string()).unwrap_or_default(),
            ]
        })
        .collect();
    write_table(
        "API Keys",
        &["Key Hash", "Alias", "User ID", "Team ID", "Spend"],
        rows,
        stdout,
    );
}

fn list_query(options: &ListKeys) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(page) = options.page {
        query.push((String::from("page"), page.to_string()));
    }
    if let Some(size) = options.size {
        query.push((String::from("size"), size.to_string()));
    }
    if let Some(user_id) = options.user_id.as_deref() {
        query.push((String::from("user_id"), user_id.to_owned()));
    }
    if let Some(team_id) = options.team_id.as_deref() {
        query.push((String::from("team_id"), team_id.to_owned()));
    }
    if let Some(organization_id) = options.organization_id.as_deref() {
        query.push((String::from("organization_id"), organization_id.to_owned()));
    }
    if let Some(key_hash) = options.key_hash.as_deref() {
        query.push((String::from("key_hash"), key_hash.to_owned()));
    }
    if let Some(key_alias) = options.key_alias.as_deref() {
        query.push((String::from("key_alias"), key_alias.to_owned()));
    }
    query.push((
        String::from("return_full_object"),
        options.return_full_object.to_string(),
    ));
    query.push((
        String::from("include_team_keys"),
        options.include_team_keys.to_string(),
    ));
    query
}
