use betterlite_mcp::gateway_authorize::contract::NativeClientAuthContract;
use betterlite_mcp::gateway_authorize::reply::{
    ConnectFlowDescription, FlowCookie, FlowCookieDeletion, GatewayReply,
};
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
        } => redirect_parts(&location, set_cookie.as_deref(), delete_cookie.as_deref()),
        GatewayReply::Html {
            body,
            headers,
            set_cookie,
            delete_cookie,
        } => html_parts(
            &body,
            headers,
            set_cookie.as_deref(),
            delete_cookie.as_deref(),
        ),
        GatewayReply::Json { body, headers } => json_parts(&body, headers),
        GatewayReply::Error(error) => error_parts(&error),
    }
}

fn redirect_parts(
    location: &str,
    set_cookie: Option<&FlowCookie>,
    delete_cookie: Option<&FlowCookieDeletion>,
) -> (u16, Value, Vec<(String, String)>, Value, Value) {
    (
        303,
        Value::String(redact(location)),
        Vec::new(),
        Value::Null,
        cookie_text(set_cookie, delete_cookie),
    )
}

fn html_parts(
    body: &str,
    headers: Vec<(String, String)>,
    set_cookie: Option<&FlowCookie>,
    delete_cookie: Option<&FlowCookieDeletion>,
) -> (u16, Value, Vec<(String, String)>, Value, Value) {
    (
        200,
        Value::Null,
        headers,
        Value::String(redact(body)),
        cookie_text(set_cookie, delete_cookie),
    )
}

fn json_parts(
    body: &ConnectFlowDescription,
    headers: Vec<(String, String)>,
) -> (u16, Value, Vec<(String, String)>, Value, Value) {
    (
        200,
        Value::Null,
        headers,
        Value::String(serde_json::to_string(body).unwrap_or_default()),
        Value::Null,
    )
}

fn error_parts(error: &OauthError) -> (u16, Value, Vec<(String, String)>, Value, Value) {
    let headers = error
        .headers()
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    (
        error.status,
        Value::Null,
        headers,
        Value::String(serde_json::to_string(&error.body()).unwrap_or_default()),
        Value::Null,
    )
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
            .find(|(header, _value)| header.eq_ignore_ascii_case(name))
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
    let tail = rest.split_once('&').map_or("", |(_code, rest)| rest);
    let mut redacted = format!("{head}code=<code>");
    if !tail.is_empty() {
        redacted.push('&');
        redacted.push_str(tail);
    }
    redacted
}
