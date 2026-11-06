use std::collections::VecDeque;
use std::future::ready;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chatgpt::login::http::{
    AuthPost, AuthResponse, AuthTransport, Body, HttpFailure,
};
use betterlite_openai::chatgpt::login::{DeviceDisplay, Pause};
use betterlite_openai::chatgpt::token_store::Clock;
use betterlite_openai::chatgpt::token_store::codec::AuthRecord;
use betterlite_openai::chatgpt::token_store::files::{FileAccess, LocalFiles, StoreEvent};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Number, Value, json};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

pub(super) type Events = Arc<Mutex<Vec<&'static str>>>;

pub(super) fn record(events: &Events, label: &'static str) {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(label);
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct Reply {
    status: Option<u16>,
    body: String,
}

pub(super) struct Files {
    pub writes: Vec<Value>,
    pub events: Events,
}

impl FileAccess for Files {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()> {
        LocalFiles.ensure_dir(path)
    }
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        LocalFiles.read(path)
    }
    fn write(&mut self, path: &Path, record: &AuthRecord) -> io::Result<()> {
        self.writes
            .push(serde_json::to_value(record).map_err(io::Error::other)?);
        LocalFiles.write(path, record)
    }
    fn report(&mut self, event: StoreEvent) {
        record(
            &self.events,
            match event {
                StoreEvent::InvalidRecord => "warning",
                StoreEvent::WriteFailure => "error",
            },
        );
    }
}

pub(super) struct Time {
    pub now: Decimal,
    pub sleeps: Vec<Number>,
    pub path: PathBuf,
    pub replacement: Option<String>,
    pub failure: Option<io::Error>,
}

impl Time {
    fn elapse(&mut self, duration: Duration) {
        let seconds = Decimal::from(duration.as_secs())
            .saturating_add(Decimal::new(i64::from(duration.subsec_nanos()), 9));
        match seconds.to_string().parse() {
            Ok(value) => self.sleeps.push(value),
            Err(error) => self.failure = Some(io::Error::other(error)),
        }
        self.now = self.now.saturating_add(seconds);
        if let Some(replacement) = self.replacement.take() {
            self.failure = std::fs::write(&self.path, replacement).err();
        }
    }
}

impl Clock for Time {
    fn now(&self) -> Decimal {
        self.now
    }
    fn sleep(&mut self, duration: Duration) {
        self.elapse(duration);
    }
}

impl Pause for Time {
    async fn pause(&mut self, duration: Duration) {
        self.elapse(duration);
    }
}

pub(super) struct Script {
    pub replies: VecDeque<Reply>,
    pub repeat: Option<Reply>,
    pub requests: Vec<Value>,
}

impl AuthTransport for Script {
    fn post(
        &mut self,
        request: AuthPost<'_>,
    ) -> impl Future<Output = Result<AuthResponse, HttpFailure>> + Send {
        let (content_type, sent) = match &request.body {
            Body::Json(text) => (
                "application/json",
                serde_json::from_str(text.expose_secret())
                    .unwrap_or_else(|_| Value::String(text.expose_secret().to_owned())),
            ),
            Body::Form(text) => (
                "application/x-www-form-urlencoded",
                Value::String(text.expose_secret().to_owned()),
            ),
        };
        self.requests
            .push(json!({ "url": request.url, "content_type": content_type, "body": sent }));
        let reply = self.replies.pop_front().or_else(|| self.repeat.clone());
        ready(match reply {
            Some(Reply {
                status: Some(status),
                body,
            }) => Ok(AuthResponse {
                status,
                body: body.into_bytes(),
            }),
            _ => Err(HttpFailure::Transport(TransportError::Closed)),
        })
    }
}

pub(super) struct Shown {
    pub prompts: Vec<String>,
}

impl DeviceDisplay for Shown {
    fn show(&mut self, verify_url: &str, user_code: &SecretString) {
        self.prompts.push(format!(
            "Sign in with ChatGPT using device code:\n1) Visit {verify_url}\n2) Enter code: {}\nDevice codes are a common phishing target. Never share this code.",
            user_code.expose_secret()
        ));
    }
}

pub(super) struct Diagnostics(pub Events);

impl<S: Subscriber> Layer<S> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        match *event.metadata().level() {
            tracing::Level::WARN => record(&self.0, "warning"),
            tracing::Level::ERROR => record(&self.0, "error"),
            _ => {}
        }
    }
}
