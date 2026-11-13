use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::database_refresh_loop::extraction::extract_refresh_password;
use betterlite_db::database_refresh_loop::schedule::{
    RefreshQuery, is_token_expired, parse_expiration, seconds_until_refresh,
    token_refresh_not_needed,
};
use betterlite_db::database_tokens::expiry::{ExpiryError, TokenAuthKind};
use chrono::NaiveDateTime;
use serde::Deserialize;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthKind {
    Disabled,
    Rds,
    Azure,
}

impl AuthKind {
    const fn token_kind(self) -> Option<TokenAuthKind> {
        match self {
            Self::Disabled => None,
            Self::Rds => Some(TokenAuthKind::Rds),
            Self::Azure => Some(TokenAuthKind::Azure),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    ExtractPassword {
        url: Option<String>,
    },
    ParseExpiration {
        auth: AuthKind,
        token: Option<String>,
    },
    SecondsUntilRefresh {
        auth: AuthKind,
        url: Option<String>,
        now: String,
    },
    IsTokenExpired {
        auth: AuthKind,
        url: Option<String>,
        now: String,
    },
    TokenRefreshNotNeeded {
        auth: AuthKind,
        url: Option<String>,
        now: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ExtractPassword {
        url: Option<String>,
    },
    ParseExpiration {
        auth: AuthKind,
        token: Option<String>,
    },
    SecondsUntilRefresh {
        auth: AuthKind,
        url: Option<String>,
        now: String,
    },
    IsTokenExpired {
        auth: AuthKind,
        url: Option<String>,
        now: String,
    },
    TokenRefreshNotNeeded {
        auth: AuthKind,
        url: Option<String>,
        now: String,
    },
    Batch {
        calls: Vec<Call>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Batch { calls } => Ok(serde_json::Value::Array(
                calls.iter().map(scenario).collect(),
            )),
            Input::ExtractPassword { url } => Ok(extract(url.as_deref())),
            Input::ParseExpiration { auth, token } => expiration(*auth, token.as_deref()),
            Input::SecondsUntilRefresh { auth, url, now } => scheduled(*auth, url.as_deref(), now),
            Input::IsTokenExpired { auth, url, now } => expired(*auth, url.as_deref(), now),
            Input::TokenRefreshNotNeeded { auth, url, now } => {
                not_needed(*auth, url.as_deref(), now)
            }
        }
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn parse_now(text: &str) -> Result<NaiveDateTime, Box<HelperError>> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .map_err(|error| helper_error("ValueError", error.to_string()))
}

fn levels_for<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let levels = Levels::default();
    let subscriber = tracing_subscriber::registry().with(levels.clone());
    let value = tracing::subscriber::with_default(subscriber, run);
    let captured = levels
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, captured)
}

fn extract(url: Option<&str>) -> serde_json::Value {
    let (password, log_levels) = levels_for(|| url.and_then(extract_refresh_password));
    serde_json::json!({"password": password, "log_levels": log_levels})
}

fn expiration(auth: AuthKind, token: Option<&str>) -> HelperResult {
    let (parsed, log_levels) = levels_for(|| parse_expiration(auth.token_kind(), token));
    let Some(expiration) =
        parsed.map_err(|error| helper_error("OverflowError", error.to_string()))?
    else {
        return Ok(serde_json::json!({"expiration": null, "log_levels": log_levels}));
    };
    Ok(serde_json::json!({
        "expiration": expiration.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "log_levels": log_levels,
    }))
}

fn capture_clock<T>(
    auth: AuthKind,
    url: Option<&str>,
    now: &str,
    run: fn(&RefreshQuery<'_>) -> Result<T, ExpiryError>,
) -> Result<(T, Vec<String>), Box<HelperError>> {
    let moment = parse_now(now)?;
    let query = RefreshQuery {
        kind: auth.token_kind(),
        token_url: url,
        now: moment,
    };
    let (outcome, log_levels) = levels_for(|| run(&query));
    let value = outcome.map_err(|error| helper_error("OverflowError", error.to_string()))?;
    Ok((value, log_levels))
}

fn scheduled(auth: AuthKind, url: Option<&str>, now: &str) -> HelperResult {
    let (schedule, log_levels) = capture_clock(auth, url, now, seconds_until_refresh)?;
    Ok(serde_json::json!({"seconds": schedule.seconds, "log_levels": log_levels}))
}

fn expired(auth: AuthKind, url: Option<&str>, now: &str) -> HelperResult {
    let (expired, log_levels) = capture_clock(auth, url, now, is_token_expired)?;
    Ok(serde_json::json!({"expired": expired, "log_levels": log_levels}))
}

fn not_needed(auth: AuthKind, url: Option<&str>, now: &str) -> HelperResult {
    let (not_needed, log_levels) = capture_clock(auth, url, now, token_refresh_not_needed)?;
    Ok(serde_json::json!({"not_needed": not_needed, "log_levels": log_levels}))
}

fn scenario(call: &Call) -> serde_json::Value {
    let outcome = match call {
        Call::ExtractPassword { url } => Ok(extract(url.as_deref())),
        Call::ParseExpiration { auth, token } => expiration(*auth, token.as_deref()),
        Call::SecondsUntilRefresh { auth, url, now } => scheduled(*auth, url.as_deref(), now),
        Call::IsTokenExpired { auth, url, now } => expired(*auth, url.as_deref(), now),
        Call::TokenRefreshNotNeeded { auth, url, now } => not_needed(*auth, url.as_deref(), now),
    };
    match outcome {
        Ok(value) => serde_json::json!({"ok": true, "value": value}),
        Err(_error) => serde_json::json!({"ok": false}),
    }
}

#[derive(Clone, Default)]
struct Levels(Arc<Mutex<Vec<String>>>);

impl<Inner: Subscriber> Layer<Inner> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.metadata().level().as_str().to_ascii_lowercase());
    }
}
