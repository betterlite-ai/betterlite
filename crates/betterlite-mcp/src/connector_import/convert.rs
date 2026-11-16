use super::request::{ConnectorEntry, ConnectorImportRequest, ConnectorServers};
use crate::server_payload::request::{NewMCPServerRequest, OrderedMap};
use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::ExposeSecret;
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
    let Some(name) = entry.name.as_deref().filter(|text| !text.is_empty()) else {
        let url = entry.url.as_deref().filter(|text| !text.is_empty());
        let label = url.map_or_else(|| format!("entry {index}"), str::to_owned);
        return failed(&label, "Connector entries in list form must have a name.");
    };
    convert_entry(name, entry, stdio)
}
fn convert_entry(name: &str, entry: &ConnectorEntry, stdio: &StdioPolicy) -> ConnectorOutcome {
    let sanitized = sanitize_connector_name(name);
    if sanitized.is_empty() {
        return failed(name, "Connector name is empty after sanitization.");
    }
    let url = entry.url.as_deref().filter(|text| !text.is_empty());
    let command = entry.command.as_deref().filter(|text| !text.is_empty());
    if url.is_some() && command.is_some() {
        return failed(name, "Connector cannot have both a url and a command.");
    }
    let mut fields = base_fields(sanitized, entry);
    match (command, url) {
        (Some(program), _) => add_stdio_fields(&mut fields, program, entry),
        (None, Some(address)) => {
            let Some(transport) = supported_transport(entry) else {
                let sent = entry.entry_type.as_deref().unwrap_or("");
                return failed(name, &format!("Unsupported connector type '{sent}'."));
            };
            fields.insert("transport".to_owned(), Json::Text(transport.to_owned()));
            fields.insert("url".to_owned(), Json::Text(address.to_owned()));
            add_remote_auth(&mut fields, entry);
        }
        (None, None) => return failed(name, "Connector must have either a url or a command."),
    }
    match NewMCPServerRequest::from_value(&Json::Object(fields), stdio) {
        Ok(request) => ConnectorOutcome::Converted(ConvertedConnector {
            name: name.to_owned(),
            request: Box::new(request),
        }),
        Err(error) => failed(name, &format!("Value error, {error}")),
    }
}
fn base_fields(sanitized: String, entry: &ConnectorEntry) -> Object {
    let mut fields = Object::new();
    fields.insert("server_name".to_owned(), Json::Text(sanitized.clone()));
    fields.insert("alias".to_owned(), Json::Text(sanitized));
    if let Some(description) = &entry.description {
        fields.insert("description".to_owned(), Json::Text(description.clone()));
    }
    fields.insert(
        "approval_status".to_owned(),
        Json::Text("active".to_owned()),
    );
    fields
}
fn add_stdio_fields(fields: &mut Object, program: &str, entry: &ConnectorEntry) {
    fields.insert("transport".to_owned(), Json::Text("stdio".to_owned()));
    fields.insert("command".to_owned(), Json::Text(program.to_owned()));
    let args = entry.args.iter().cloned().map(Json::Text).collect();
    fields.insert("args".to_owned(), Json::List(args));
    fields.insert("env".to_owned(), map_json(&entry.env));
}
fn add_remote_auth(fields: &mut Object, entry: &ConnectorEntry) {
    let (header_value, static_headers) = split_headers(entry.headers.as_ref());
    let direct = entry
        .authorization_token
        .as_ref()
        .map(ExposeSecret::expose_secret)
        .filter(|text| !text.is_empty());
    let header = header_value.as_deref();
    let bearer = header.and_then(bearer_token);
    let (auth_type, auth_value) = match (direct, bearer, header) {
        (Some(token), _, _) | (None, Some(token), _) => ("bearer_token", Some(token)),
        (None, None, Some(whole)) => ("authorization", Some(whole)),
        (None, None, None) => ("none", None),
    };
    fields.insert("auth_type".to_owned(), Json::Text(auth_type.to_owned()));
    if let Some(value) = auth_value {
        let credential = Json::Text(value.to_owned());
        let credentials = Object::from([("auth_value".to_owned(), credential)]);
        fields.insert("credentials".to_owned(), Json::Object(credentials));
    }
    if let Some(headers) = static_headers {
        fields.insert("static_headers".to_owned(), map_json(&headers));
    }
}
fn split_headers(headers: Option<&OrderedMap>) -> (Option<String>, Option<OrderedMap>) {
    let Some(all) = headers else {
        return (None, None);
    };
    let mut authorization = None;
    let mut seen = false;
    let mut remaining = OrderedMap::new();
    for (name, value) in all {
        if name.eq_ignore_ascii_case("authorization") {
            if !seen {
                authorization = (!value.is_empty()).then(|| value.clone());
            }
            seen = true;
        } else {
            remaining.insert(name.clone(), value.clone());
        }
    }
    (authorization, (!remaining.is_empty()).then_some(remaining))
}
fn bearer_token(value: &str) -> Option<&str> {
    let prefix = value.get(..7)?;
    if !prefix.eq_ignore_ascii_case("bearer ") {
        return None;
    }
    value.get(7..)
}
fn supported_transport(entry: &ConnectorEntry) -> Option<&'static str> {
    let lowered = entry.entry_type.as_deref().unwrap_or("").to_lowercase();
    match lowered.as_str() {
        "sse" => Some("sse"),
        "url" | "http" | "streamable_http" | "streamable-http" | "" => Some("http"),
        _ => None,
    }
}
fn map_json(values: &OrderedMap) -> Json {
    let fields = values
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())));
    Json::Object(fields.collect())
}
fn failed(name: &str, error: &str) -> ConnectorOutcome {
    ConnectorOutcome::Failed(ConnectorConversionError {
        name: name.to_owned(),
        error: error.to_owned(),
    })
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
