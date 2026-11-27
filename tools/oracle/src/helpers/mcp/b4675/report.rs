use betterlite_mcp::gateway_authorize::contract::NativeClientAuthContract;
use betterlite_mcp::gateway_authorize::reply::{FlowCookie, FlowCookieDeletion, GatewayReply};
use betterlite_mcp::gateway_dcr::oauth_error::OauthError;
use betterlite_mcp::gateway_dcr::oauth_error::OauthErrorResponse as _;
use serde_json::{Value, json};

const PAGE_HEADERS: [(&str, &str); 4] = [
    ("cache-control", "no-store"),
    ("content-security-policy", "frame-ancestors 'none'"),
    ("pragma", "no-cache"),
    ("x-frame-options", "DENY"),
];

pub(super) fn step_report(name: &str, reply: GatewayReply) -> (Value, Option<String>) {
    let captured = captured_cookie(&reply);
    let (status, location, headers, body, cookie) = parts(reply);
    let report = json!({
        "name": name,
        "status": status,
        "location": location,
        "headers": header_pairs(&headers),
        "body": body,
        "cookie": cookie,
    });
    (report, captured)
}

fn captured_cookie(reply: &GatewayReply) -> Option<String> {
    match reply {
        GatewayReply::Redirect { set_cookie, .. } | GatewayReply::Html { set_cookie, .. } => {
            set_cookie
                .as_ref()
                .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        }
        GatewayReply::Json { .. } | GatewayReply::Error(_) => None,
    }
}

fn parts(reply: GatewayReply) -> (u16, Value, Vec<(String, String)>, Value, Value) {
    match reply {
        GatewayReply::Redirect {
            location,
            set_cookie,
            delete_cookie,
        } => (
            303,
            Value::String(redact(&location)),
            Vec::new(),
            Value::Null,
            cookie_text(set_cookie.as_deref(), delete_cookie.as_deref()),
        ),
        GatewayReply::Html {
            body,
            headers,
            set_cookie,
            delete_cookie,
        } => (
            200,
            Value::Null,
            headers,
            Value::String(redact(&body)),
            cookie_text(set_cookie.as_deref(), delete_cookie.as_deref()),
        ),
        GatewayReply::Json { body, headers } => (
            200,
            Value::Null,
            headers,
            Value::String(serde_json::to_string(&body).unwrap_or_default()),
            Value::Null,
        ),
        GatewayReply::Error(error) => (
            error.status,
            Value::Null,
            error_headers(&error),
            Value::String(serde_json::to_string(&error.body()).unwrap_or_default()),
            Value::Null,
        ),
    }
}

fn error_headers(error: &OauthError) -> Vec<(String, String)> {
    error
        .headers()
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn contract_report(name: &str, contract: &NativeClientAuthContract) -> Value {
    json!({
        "name": name,
        "status": 200,
        "location": Value::Null,
        "headers": Value::Array(Vec::new()),
        "body": serde_json::to_string(contract).unwrap_or_default(),
        "cookie": Value::Null,
    })
}

fn header_pairs(headers: &[(String, String)]) -> Value {
    let mut pairs = Vec::new();
    for &(name, _value) in &PAGE_HEADERS {
        if let Some((_found, value)) = headers
            .iter()
            .find(|(header, _cell)| header.eq_ignore_ascii_case(name))
        {
            pairs.push(json!([name, value]));
        }
    }
    Value::Array(pairs)
}

fn cookie_text(set: Option<&FlowCookie>, delete: Option<&FlowCookieDeletion>) -> Value {
    if let Some(cookie) = set {
        return Value::String(format!(
            "{}|{}|{}|{}|true|lax",
            cookie.name, cookie.max_age_seconds, cookie.path, cookie.secure
        ));
    }
    if let Some(cookie) = delete {
        return Value::String(format!(
            "{}|0|{}|{}|true|lax",
            cookie.name, cookie.path, cookie.secure
        ));
    }
    Value::Null
}

fn redact(text: &str) -> String {
    let Some((head, rest)) = text.split_once("code=") else {
        return text.to_owned();
    };
    let tail = rest.split_once('&').map_or("", |(_code, after)| after);
    let mut redacted = format!("{head}code=<code>");
    if !tail.is_empty() {
        redacted.push('&');
        redacted.push_str(tail);
    }
    redacted
}
