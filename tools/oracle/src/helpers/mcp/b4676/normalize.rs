use betterlite_mcp::gateway_dcr::oauth_error::{OauthError, OauthErrorResponse as _};
use betterlite_mcp::gateway_token::support::reply::{
    IntrospectionBody, ProxyCredentialBody, TokenBody, TokenReply,
};
use betterlite_mcp::session_tokens::open_session_token;
use betterlite_mcp::types::{OpenedSessionToken, SessionSigningKeys, SessionTokenKind};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

const REPORT_HEADERS: [&str; 2] = ["cache-control", "pragma"];

pub(super) fn token_report(
    name: &str,
    reply: &TokenReply,
    keys: Option<&SessionSigningKeys>,
    now: DateTime<Utc>,
    clients: &[String],
) -> Value {
    match reply {
        TokenReply::Json {
            status,
            body,
            headers,
        } => {
            let pairs = string_pairs(headers);
            json!({
                "name": name,
                "status": status,
                "headers": header_pairs(&pairs),
                "body": body_value(body, keys, now, clients),
            })
        }
        TokenReply::Error(error) => {
            let pairs = error_pairs(error);
            json!({
                "name": name,
                "status": error.status,
                "headers": header_pairs(&pairs),
                "body": serde_json::to_value(error.body()).unwrap_or_default(),
            })
        }
    }
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

fn error_pairs(error: &OauthError) -> Vec<(&str, &str)> {
    let headers = error.headers();
    headers.into_iter().collect()
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

fn body_value(
    body: &TokenBody,
    keys: Option<&SessionSigningKeys>,
    now: DateTime<Utc>,
    clients: &[String],
) -> Value {
    match body {
        TokenBody::SessionPair(pair) => json!({
            "access_token": claims(&pair.access_token, SessionTokenKind::Session, keys, now, clients),
            "token_type": pair.token_type,
            "expires_in": pair.expires_in,
            "refresh_token": claims(&pair.refresh_token, SessionTokenKind::Refresh, keys, now, clients),
        }),
        TokenBody::ProxyCredential(credential) => proxy_value(credential, keys, now, clients),
        TokenBody::Revoked(_revoked) => json!({}),
        TokenBody::Introspection(body) => introspection_value(body, clients),
    }
}

fn proxy_value(
    credential: &ProxyCredentialBody,
    keys: Option<&SessionSigningKeys>,
    now: DateTime<Utc>,
    clients: &[String],
) -> Value {
    let refresh = claims(
        &credential.refresh_token,
        SessionTokenKind::Refresh,
        keys,
        now,
        clients,
    );
    match credential.issued_token_type {
        None => json!({
            "access_token": credential.access_token,
            "token_type": credential.token_type,
            "expires_in": credential.expires_in,
            "refresh_token": refresh,
            "user_id": credential.user_id,
            "team_id": credential.team_id,
        }),
        Some(issued) => json!({
            "access_token": credential.access_token,
            "token_type": credential.token_type,
            "expires_in": credential.expires_in,
            "refresh_token": refresh,
            "user_id": credential.user_id,
            "team_id": credential.team_id,
            "issued_token_type": issued,
        }),
    }
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

fn claims(
    token: &str,
    kind: SessionTokenKind,
    keys: Option<&SessionSigningKeys>,
    now: DateTime<Utc>,
    clients: &[String],
) -> Value {
    let Some(keys) = keys else {
        return json!("<unopenable>");
    };
    match open_session_token(token, kind, keys, now) {
        Ok(opened) => opened_value(&opened, clients),
        Err(_error) => json!("<unopenable>"),
    }
}

fn opened_value(opened: &OpenedSessionToken, clients: &[String]) -> Value {
    let principal = &opened.principal;
    json!({
        "user_id": principal.user_id,
        "client_id": client_label(&principal.client_id, clients),
        "resource_server_id": principal.resource_server_id,
        "audience": principal.audience,
        "kind": opened.kind,
        "iat": opened.iat,
        "exp": opened.exp,
    })
}

fn client_label(client_id: &str, clients: &[String]) -> Value {
    match clients
        .iter()
        .position(|sealed| sealed.as_str() == client_id)
    {
        Some(index) => json!({"sealed_client": index}),
        None => json!(client_id),
    }
}
