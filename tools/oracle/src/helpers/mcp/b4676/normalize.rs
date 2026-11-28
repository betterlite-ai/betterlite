use betterlite_mcp::gateway_dcr::oauth_error::OauthErrorResponse as _;
use betterlite_mcp::gateway_token::support::reply::{
    IntrospectionBody, ProxyCredentialBody, SessionPairBody, TokenBody, TokenReply,
};
use betterlite_mcp::session_tokens::open_session_token;
use betterlite_mcp::types::{SessionSigningKeys, SessionTokenKind};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

const REPORT_HEADERS: [&str; 2] = ["cache-control", "pragma"];

struct Snapshot<'snapshot> {
    keys: Option<&'snapshot SessionSigningKeys>,
    now: DateTime<Utc>,
    clients: &'snapshot [String],
}

pub(super) fn token_report(
    name: &str,
    reply: &TokenReply,
    keys: Option<&SessionSigningKeys>,
    now: DateTime<Utc>,
    clients: &[String],
) -> Value {
    let view = Snapshot { keys, now, clients };
    let (status, body, headers) = match reply {
        TokenReply::Json {
            status,
            body,
            headers,
        } => (*status, body_value(body, &view), string_pairs(headers)),
        TokenReply::Error(error) => (
            error.status,
            serde_json::to_value(error.body()).unwrap_or_default(),
            error.headers().into_iter().collect(),
        ),
    };
    json!({
        "name": name,
        "status": status,
        "headers": header_pairs(&headers),
        "body": body,
    })
}

pub(super) fn matches_report(name: &str, matched: bool) -> Value {
    json!({
        "name": name,
        "status": 200,
        "headers": [],
        "body": {"matches": matched},
    })
}

fn string_pairs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

fn header_pairs(headers: &[(&str, &str)]) -> Value {
    let mut pairs = Vec::new();
    for name in REPORT_HEADERS {
        let found = headers
            .iter()
            .copied()
            .find(|(header, _value)| header.eq_ignore_ascii_case(name));
        if let Some((_header, value)) = found {
            pairs.push(json!([name, value]));
        }
    }
    Value::Array(pairs)
}

fn body_value(body: &TokenBody, view: &Snapshot<'_>) -> Value {
    match body {
        TokenBody::SessionPair(pair) => session_pair_value(pair, view),
        TokenBody::ProxyCredential(credential) => proxy_value(credential, view),
        TokenBody::Revoked(_revoked) => json!({}),
        TokenBody::Introspection(introspection) => introspection_value(introspection, view.clients),
    }
}

fn session_pair_value(pair: &SessionPairBody, view: &Snapshot<'_>) -> Value {
    json!({
        "access_token": claims(&pair.access_token, SessionTokenKind::Session, view),
        "token_type": pair.token_type,
        "expires_in": pair.expires_in,
        "refresh_token": claims(&pair.refresh_token, SessionTokenKind::Refresh, view),
    })
}

fn proxy_value(credential: &ProxyCredentialBody, view: &Snapshot<'_>) -> Value {
    let mut value = serde_json::to_value(credential).unwrap_or_default();
    if let Value::Object(fields) = &mut value {
        let refresh = claims(&credential.refresh_token, SessionTokenKind::Refresh, view);
        fields.insert("refresh_token".to_owned(), refresh);
        if let Some(issued) = credential.issued_token_type {
            fields.insert("issued_token_type".to_owned(), json!(issued));
        }
    }
    value
}

fn introspection_value(body: &IntrospectionBody, clients: &[String]) -> Value {
    let mut value = serde_json::to_value(body).unwrap_or_default();
    if let Value::Object(fields) = &mut value {
        if fields.contains_key("jti") {
            fields.insert("jti".to_owned(), json!("<jti>"));
        }
        if let Some(Value::String(client_id)) = fields.get("client_id") {
            let label = client_label(client_id, clients);
            fields.insert("client_id".to_owned(), label);
        }
    }
    value
}

fn claims(token: &str, kind: SessionTokenKind, view: &Snapshot<'_>) -> Value {
    let Some(keys) = view.keys else {
        return json!("<unopenable>");
    };
    let Ok(opened) = open_session_token(token, kind, keys, view.now) else {
        return json!("<unopenable>");
    };
    let principal = &opened.principal;
    json!({
        "user_id": principal.user_id,
        "client_id": client_label(&principal.client_id, view.clients),
        "resource_server_id": principal.resource_server_id,
        "audience": principal.audience,
        "kind": opened.kind,
        "iat": opened.iat,
        "exp": opened.exp,
    })
}

fn client_label(client_id: &str, clients: &[String]) -> Value {
    clients
        .iter()
        .position(|sealed| sealed.as_str() == client_id)
        .map_or_else(|| json!(client_id), |index| json!({"sealed_client": index}))
}
