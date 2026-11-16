use betterlite_prompts::arguments::json::{Json, Object};

use crate::server_payload::request::{NewMCPServerRequest, OrderedMap};
use crate::server_payload::rules::StdioPolicy;

use super::request::{ConnectorEntry, ConnectorImportRequest, ConnectorServers};

mod tests;

#[derive(Debug, Clone)]
pub enum ConnectorOutcome {
    Converted(ConvertedConnector),
    Failed(ConnectorConversionError),
}

#[derive(Debug, Clone)]
pub struct ConvertedConnector {
    pub name: String,
    pub request: Box<NewMCPServerRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorConversionError {
    pub name: String,
    pub error: String,
}

#[must_use]
pub fn convert_connector_entries(
    request: &ConnectorImportRequest,
    stdio: &StdioPolicy,
) -> Vec<ConnectorOutcome> {
    match &request.mcp_servers {
        ConnectorServers::Mapping(entries) => entries
            .iter()
            .map(|(name, entry)| convert_entry(name, entry, stdio))
            .collect(),
        ConnectorServers::List(entries) => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| convert_list_entry(index, entry, stdio))
            .collect(),
    }
}

fn convert_list_entry(
    index: usize,
    entry: &ConnectorEntry,
    stdio: &StdioPolicy,
) -> ConnectorOutcome {
    let Some(name) = non_empty(entry.name.as_deref()) else {
        let name =
            non_empty(entry.url.as_deref()).map_or_else(|| format!("entry {index}"), str::to_owned);
        return failed(&name, "Connector entries in list form must have a name.");
    };
    convert_entry(name, entry, stdio)
}

fn convert_entry(name: &str, entry: &ConnectorEntry, stdio: &StdioPolicy) -> ConnectorOutcome {
    let sanitized = sanitize_connector_name(name);
    if sanitized.is_empty() {
        return failed(name, "Connector name is empty after sanitization.");
    }
    let url = non_empty(entry.url.as_deref());
    let command = non_empty(entry.command.as_deref());
    if url.is_some() && command.is_some() {
        return failed(name, "Connector cannot have both a url and a command.");
    }
    if let Some(command) = command {
        return stdio_outcome(name, &sanitized, command, entry, stdio);
    }
    let Some(url) = url else {
        return failed(name, "Connector must have either a url or a command.");
    };
    remote_outcome(name, &sanitized, url, entry, stdio)
}

fn stdio_outcome(
    name: &str,
    sanitized: &str,
    command: &str,
    entry: &ConnectorEntry,
    stdio: &StdioPolicy,
) -> ConnectorOutcome {
    let payload = stdio_payload(sanitized, command, entry);
    build_outcome(name, payload, stdio)
}

fn remote_outcome(
    name: &str,
    sanitized: &str,
    url: &str,
    entry: &ConnectorEntry,
    stdio: &StdioPolicy,
) -> ConnectorOutcome {
    let Some(transport) = supported_transport(entry) else {
        let sent = entry.entry_type.as_deref().unwrap_or("");
        return failed(name, &format!("Unsupported connector type '{sent}'."));
    };
    let auth = remote_auth(entry);
    let payload = remote_payload(sanitized, url, transport, entry, &auth);
    build_outcome(name, payload, stdio)
}

fn build_outcome(name: &str, payload: Json, stdio: &StdioPolicy) -> ConnectorOutcome {
    match NewMCPServerRequest::from_value(payload, stdio) {
        Ok(request) => ConnectorOutcome::Converted(ConvertedConnector {
            name: name.to_owned(),
            request: Box::new(request),
        }),
        Err(error) => failed(name, &format!("Value error, {error}")),
    }
}

fn failed(name: &str, error: &str) -> ConnectorOutcome {
    ConnectorOutcome::Failed(ConnectorConversionError {
        name: name.to_owned(),
        error: error.to_owned(),
    })
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    let text = value?;
    if text.is_empty() {
        return None;
    }
    Some(text)
}

fn supported_transport(entry: &ConnectorEntry) -> Option<&'static str> {
    let lowered = entry.entry_type.as_deref().unwrap_or("").to_lowercase();
    match lowered.as_str() {
        "sse" => Some("sse"),
        "url" | "http" | "streamable_http" | "streamable-http" | "" => Some("http"),
        _ => None,
    }
}

struct RemoteAuth {
    auth_type: &'static str,
    credentials: Option<Json>,
    static_headers: Option<OrderedMap>,
}

fn remote_auth(entry: &ConnectorEntry) -> RemoteAuth {
    let static_headers = remaining_headers(entry.headers.as_ref());
    if let Some(token) = non_empty(entry.authorization_token.as_deref()) {
        return bearer_auth(token, static_headers);
    }
    let Some(header_value) = authorization_header(entry.headers.as_ref()) else {
        return RemoteAuth {
            auth_type: "none",
            credentials: None,
            static_headers,
        };
    };
    if let Some(token) = bearer_token(&header_value) {
        return bearer_auth(token, static_headers);
    }
    RemoteAuth {
        auth_type: "authorization",
        credentials: Some(auth_value(&header_value)),
        static_headers,
    }
}

fn authorization_header(headers: Option<&OrderedMap>) -> Option<String> {
    let headers = headers?;
    let value = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.clone())?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn remaining_headers(headers: Option<&OrderedMap>) -> Option<OrderedMap> {
    let mut remaining = OrderedMap::new();
    if let Some(headers) = headers {
        for (name, value) in headers {
            if !name.eq_ignore_ascii_case("authorization") {
                remaining.insert(name.clone(), value.clone());
            }
        }
    }
    if remaining.is_empty() {
        return None;
    }
    Some(remaining)
}

fn bearer_token(value: &str) -> Option<&str> {
    if value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer "))
    {
        return value.get(7..);
    }
    None
}

fn bearer_auth(token: &str, static_headers: Option<OrderedMap>) -> RemoteAuth {
    let credentials = auth_value(token);
    RemoteAuth {
        auth_type: "bearer_token",
        credentials: Some(credentials),
        static_headers,
    }
}

fn auth_value(token: &str) -> Json {
    let mut fields = Object::new();
    fields.insert("auth_value".to_owned(), Json::Text(token.to_owned()));
    Json::Object(fields)
}

fn stdio_payload(sanitized: &str, command: &str, entry: &ConnectorEntry) -> Json {
    let mut fields = Object::new();
    insert_text(&mut fields, "server_name", Some(sanitized));
    insert_text(&mut fields, "alias", Some(sanitized));
    insert_text(&mut fields, "description", entry.description.as_deref());
    insert_text(&mut fields, "approval_status", Some("active"));
    insert_text(&mut fields, "transport", Some("stdio"));
    insert_text(&mut fields, "command", Some(command));
    fields.insert(
        "args".to_owned(),
        Json::List(entry.args.iter().cloned().map(Json::Text).collect()),
    );
    fields.insert("env".to_owned(), ordered_map_json(&entry.env));
    Json::Object(fields)
}

fn remote_payload(
    sanitized: &str,
    url: &str,
    transport: &str,
    entry: &ConnectorEntry,
    auth: &RemoteAuth,
) -> Json {
    let mut fields = Object::new();
    insert_text(&mut fields, "server_name", Some(sanitized));
    insert_text(&mut fields, "alias", Some(sanitized));
    insert_text(&mut fields, "description", entry.description.as_deref());
    insert_text(&mut fields, "approval_status", Some("active"));
    insert_text(&mut fields, "transport", Some(transport));
    insert_text(&mut fields, "url", Some(url));
    insert_text(&mut fields, "auth_type", Some(auth.auth_type));
    if let Some(credentials) = &auth.credentials {
        fields.insert("credentials".to_owned(), credentials.clone());
    }
    if let Some(headers) = &auth.static_headers {
        fields.insert("static_headers".to_owned(), ordered_map_json(headers));
    }
    Json::Object(fields)
}

fn ordered_map_json(values: &OrderedMap) -> Json {
    let mut fields = Object::new();
    for (name, value) in values {
        fields.insert(name.clone(), Json::Text(value.clone()));
    }
    Json::Object(fields)
}

fn insert_text(fields: &mut Object, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        fields.insert(key.to_owned(), Json::Text(value.to_owned()));
    }
}

#[must_use]
pub fn sanitize_connector_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for character in name.trim().chars() {
        let mapped = if character.is_ascii_alphanumeric() || character == '_' {
            character
        } else {
            '_'
        };
        if mapped == '_' && sanitized.ends_with('_') {
            continue;
        }
        sanitized.push(mapped);
    }
    sanitized.trim_matches('_').to_owned()
}
