mod tests;

use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_key_lookup::{KeyAuthenticator, KeyLookupOutcome};
use betterlite_mcp::byok_consent::ConsentError;
use betterlite_mcp::byok_consent::get::{GetQuery, authorize_get};
use betterlite_mcp::byok_consent::post::{Accepted, PostForm, authorize_post};
use betterlite_mcp::byok_consent::session::authenticate_session;
use betterlite_mcp::byok_consent::store::AuthCodeStore;
use betterlite_mcp::byok_token_exchange::ports::{AuthCodeLookup as _, AuthCodeRecord};
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::session_admission::auth::SessionAuthError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::Utc;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
const JSON_CONTENT_TYPE: &str = "application/json";
const ACCESS_ITEM_OPEN: &str = "class=\"access-item\"><span class=\"check\">&#10003;</span>";
const DURATION_OPEN: &str = "selDur('";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Route {
    Get,
    Post,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    route: Route,
    #[serde(default)]
    query: Option<BTreeMap<String, String>>,
    #[serde(default)]
    servers: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    form: Option<FormInput>,
    #[serde(default)]
    cookie_header: Option<String>,
    #[serde(default)]
    master_key: Option<String>,
    now: f64,
    #[serde(default)]
    prefill_live: usize,
    #[serde(default)]
    prefill_expired: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FormInput {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: String,
    server_id: String,
    api_key: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.route {
            Route::Get => get_report(input.query.as_ref(), input.servers.as_deref()),
            Route::Post => {
                let Some(form) = input.form.as_ref() else {
                    return Err(Box::new(HelperError {
                        class: "ValueError".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: "a post case needs a form".to_owned(),
                    }));
                };
                post_report(&PostCall {
                    form,
                    cookie_header: input.cookie_header.as_deref(),
                    master_key: input.master_key.as_deref(),
                    now: input.now,
                    prefill_live: input.prefill_live,
                    prefill_expired: input.prefill_expired,
                })
            }
        }
    }
}

struct PostCall<'call> {
    form: &'call FormInput,
    cookie_header: Option<&'call str>,
    master_key: Option<&'call str>,
    now: f64,
    prefill_live: usize,
    prefill_expired: usize,
}

fn get_report(
    fields: Option<&BTreeMap<String, String>>,
    servers: Option<&[serde_json::Value]>,
) -> HelperResult {
    let fields = fields.cloned().unwrap_or_default();
    let registry = match servers {
        Some(servers) => Some(registry_of(servers)?),
        None => None,
    };
    match authorize_get(&query_of(&fields), registry.as_ref()) {
        Ok(html) => Ok(json!({
            "status": 200_u16,
            "content_type": HTML_CONTENT_TYPE,
            "body": null,
            "page": page_facts(&html),
            "location": null,
            "code_is_uuid": null,
            "stored": null,
            "store_size": null,
        })),
        Err(error) => error_report(&error, None),
    }
}

fn query_of(fields: &BTreeMap<String, String>) -> GetQuery {
    let mut query = GetQuery::default();
    for (name, value) in fields {
        query.set(name, value.clone());
    }
    query
}

fn registry_of(servers: &[serde_json::Value]) -> Result<ServerRegistry, Box<HelperError>> {
    let mut parsed = Vec::with_capacity(servers.len());
    for server in servers {
        parsed.push(serde_json::from_value::<MCPServer>(server.clone()).or_value_error()?);
    }
    Ok(ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: false,
            internal_ip_ranges: None,
        },
        parsed,
        Vec::new(),
    ))
}

fn error_report(error: &ConsentError, store_size: Option<usize>) -> HelperResult {
    Ok(json!({
        "status": error.status().as_u16(),
        "content_type": JSON_CONTENT_TYPE,
        "body": serde_json::to_value(error.body()).or_value_error()?,
        "page": null,
        "location": null,
        "code_is_uuid": null,
        "stored": null,
        "store_size": store_size,
    }))
}

fn post_report(call: &PostCall<'_>) -> HelperResult {
    let codes = AuthCodeStore::new();
    prefill(&codes, call.now, call.prefill_live, call.prefill_expired);
    let Some(user_id) = admitted_user(call)? else {
        return error_report(&ConsentError::LoginRequired, Some(codes.count()));
    };
    match authorize_post(&post_form(call.form), &user_id, &codes, call.now) {
        Ok(accepted) => accepted_report(&codes, &accepted, call.now),
        Err(error) => error_report(&error, Some(codes.count())),
    }
}

fn admitted_user(call: &PostCall<'_>) -> Result<Option<String>, Box<HelperError>> {
    let master_key = call.master_key.map(SecretString::from);
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let result = runtime.block_on(authenticate_session(
        call.cookie_header,
        master_key.as_ref(),
        Some(&MissingKeys),
        Utc::now(),
    ));
    match result {
        Ok(user_id) => Ok(Some(user_id)),
        Err(SessionAuthError::LoginRequired) => Ok(None),
        Err(SessionAuthError::Construct(error)) => Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

fn post_form(form: &FormInput) -> PostForm {
    PostForm {
        client_id: form.client_id.clone(),
        redirect_uri: form.redirect_uri.clone(),
        code_challenge: form.code_challenge.clone(),
        code_challenge_method: form.code_challenge_method.clone(),
        state: form.state.clone(),
        server_id: form.server_id.clone(),
        api_key: SecretString::from(form.api_key.as_str()),
    }
}

fn accepted_report(codes: &AuthCodeStore, accepted: &Accepted, now: f64) -> HelperResult {
    let Some(record) = codes.get(&accepted.code) else {
        return Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "the accepted code was not stored".to_owned(),
        }));
    };
    Ok(json!({
        "status": 302_u16,
        "content_type": null,
        "body": null,
        "page": null,
        "location": accepted.location.replace(&accepted.code, "<code>"),
        "code_is_uuid": is_uuid4(&accepted.code),
        "stored": {
            "api_key": record.api_key.expose_secret(),
            "server_id": record.server_id,
            "code_challenge": record.code_challenge,
            "redirect_uri": record.redirect_uri,
            "client_id": record.client_id,
            "user_id": record.user_id,
            "expires_in": record.expires_at - now,
        },
        "store_size": codes.count(),
    }))
}

fn prefill(codes: &AuthCodeStore, now: f64, live: usize, expired: usize) {
    for index in 0..live {
        codes.insert(format!("live-{index}"), prefill_record(now + 300.0));
    }
    for index in 0..expired {
        codes.insert(format!("expired-{index}"), prefill_record(now - 1000.0));
    }
}

fn prefill_record(expires_at: f64) -> AuthCodeRecord {
    AuthCodeRecord {
        api_key: SecretString::from("sk-prefill"),
        server_id: String::new(),
        code_challenge: String::new(),
        redirect_uri: String::new(),
        client_id: String::new(),
        user_id: String::new(),
        expires_at,
    }
}

fn is_uuid4(code: &str) -> bool {
    code.len() == 36
        && code.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            14 => byte == b'4',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
}

fn page_facts(html: &str) -> serde_json::Value {
    json!({
        "title": between(html, "<title>", "</title>"),
        "heading": between(html, "<h2 class=\"step-title\">", "<"),
        "initial": between(html, "class=\"logo logo-s\">", "<"),
        "hidden": json!({
            "client_id": hidden_value(html, "client_id"),
            "redirect_uri": hidden_value(html, "redirect_uri"),
            "code_challenge": hidden_value(html, "code_challenge"),
            "code_challenge_method": hidden_value(html, "code_challenge_method"),
            "state": hidden_value(html, "state"),
            "server_id": hidden_value(html, "server_id"),
            "duration": hidden_value(html, "duration"),
        }),
        "access_items": access_texts(html),
        "help_href": help_href(html),
        "durations": durations(html),
        "steps": json!({
            "s1": html.contains("id=\"s1\""),
            "s2": html.contains("id=\"s2\""),
        }),
        "api_key_required": html.contains("name=\"api_key\"") && html.contains("required"),
        "form_post": html.contains("method=\"POST\""),
    })
}

fn between<'text>(text: &'text str, start: &str, end: &str) -> Option<&'text str> {
    let (_, rest) = text.split_once(start)?;
    let (found, _) = rest.split_once(end)?;
    Some(found)
}

fn hidden_value<'page>(html: &'page str, field: &str) -> Option<&'page str> {
    let marker = format!("name=\"{field}\"");
    let (_, after_name) = html.split_once(marker.as_str())?;
    between(after_name, "value=\"", "\"")
}

fn help_href(html: &str) -> Option<&str> {
    let (_, after_class) = html.split_once("class=\"help-link\"")?;
    between(after_class, "href=\"", "\"")
}

fn access_texts(html: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some((_, after_open)) = rest.split_once(ACCESS_ITEM_OPEN) {
        let Some((text, tail)) = after_open.split_once("</div>") else {
            break;
        };
        found.push(text);
        rest = tail;
    }
    found
}

fn durations(html: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some((_, after_call)) = rest.split_once(DURATION_OPEN) {
        let Some((value, tail)) = after_call.split_once('\'') else {
            break;
        };
        found.push(value);
        rest = tail;
    }
    found
}

struct MissingKeys;

#[async_trait]
impl KeyAuthenticator for MissingKeys {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        Err(KeyLookupError::MissingKey {
            key: presented.to_owned(),
        })
    }
}
