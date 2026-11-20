use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::jwt_claims::TypedValue;
use betterlite_mcp::sso_assertions::assertion::{
    SsoIdentityAssertion, assertion_expired, assertion_from_sso_login, decode_payload,
};
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Login {
        id_token: Option<TypedValue>,
        refresh_token: Option<TypedValue>,
    },
    Expiry {
        expires_at: Option<String>,
        now: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let captured = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(captured.clone());
        let result = with_default(subscriber, || answer(input))?;
        Ok(json!({"result": result, "warnings": captured.messages()}))
    }
}

fn answer(input: &Input) -> HelperResult {
    match input {
        Input::Login {
            id_token,
            refresh_token,
        } => {
            let found = assertion_from_sso_login(id_token.as_ref(), refresh_token.as_ref());
            let value = found.as_ref().map_or_else(
                || {
                    json!({
                        "id_token": null,
                        "refresh_token": null,
                        "issuer": null,
                        "expires_at": null,
                    })
                },
                |assertion| {
                    json!({
                        "id_token": assertion.id_token.expose_secret(),
                        "refresh_token": assertion.refresh_token.as_ref().map(ExposeSecret::expose_secret),
                        "issuer": assertion.issuer.clone(),
                        "expires_at": assertion.expires_at.map(|moment| moment.to_rfc3339()),
                    })
                },
            );
            Ok(value)
        }
        Input::Expiry { expires_at, now } => {
            let assertion = stored_assertion(expires_at.as_deref())?;
            let clock = DateTime::parse_from_rfc3339(now)
                .map(|moment| moment.with_timezone(&Utc))
                .or_value_error()?;
            Ok(json!({"expired": assertion_expired(&assertion, clock)}))
        }
    }
}

// the expiry step reads its bound through the stored payload decoder, the one entry a row's text takes
fn stored_assertion(expires_at: Option<&str>) -> Result<SsoIdentityAssertion, Box<HelperError>> {
    let payload = expires_at.map_or_else(
        || json!({"id_token": "id-token"}),
        |text| json!({"id_token": "id-token", "expires_at": text}),
    );
    decode_payload(&payload.to_string()).or_value_error()
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(field.0);
    }
}
