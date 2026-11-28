use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::client_auth_headers::lookup::ServerAuthHeaders;
use crate::server_record::MCPServer;

pub(crate) fn headers(entries: &[(&str, &str, &str)]) -> ServerAuthHeaders {
    let mut servers: ServerAuthHeaders = BTreeMap::new();
    for (alias, name, value) in entries {
        servers
            .entry((*alias).to_owned())
            .or_default()
            .insert((*name).to_owned(), SecretString::from((*value).to_owned()));
    }
    servers
}

pub(crate) fn oauth2(entries: &[(&str, &str)]) -> BTreeMap<String, SecretString> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from((*value).to_owned())))
        .collect()
}

pub(crate) fn groups(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub(crate) fn exposed(map: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    map.iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

#[derive(Serialize)]
struct ServerRecord<'record> {
    server_id: &'static str,
    name: &'static str,
    transport: &'static str,
    alias: Option<&'record str>,
    server_name: Option<&'record str>,
    access_groups: &'record [String],
}

pub(crate) fn server(
    alias: Option<&str>,
    server_name: Option<&str>,
    access_groups: &[&str],
) -> MCPServer {
    let groups = groups(access_groups);
    let record = ServerRecord {
        server_id: "test-server",
        name: "test-server",
        transport: "http",
        alias,
        server_name,
        access_groups: &groups,
    };
    serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap()
}
