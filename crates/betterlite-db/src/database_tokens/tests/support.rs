#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::database_tokens::provider::{
    AzureProviderCache, AzureProviderError, AzureProviderFactory, AzureTokenProvider,
    RdsTokenProvider,
};
use crate::database_tokens::settings::TokenToggles;
use crate::database_tokens::strategy::TokenProviders;
use crate::database_tokens::urls::{build_reader_url, build_writer_url};
use crate::database_urls::settings::{DatabaseUrlSettings, from_env};
use crate::rds_credentials::RdsEndpoint;
use crate::rds_credentials::error::RdsCredentialsError;

pub(super) const AZURE_RAW_TOKEN: &str = "header.claims+signature/value";

fn token_queue(tokens: &[&str]) -> Arc<Mutex<VecDeque<String>>> {
    Arc::new(Mutex::new(
        tokens.iter().map(|token| (*token).to_owned()).collect(),
    ))
}

pub(super) fn settings(pairs: &[(&str, &str)]) -> DatabaseUrlSettings {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    from_env(&environment).expect("the database settings load")
}

#[derive(Clone, Default)]
pub(super) struct RecordingRds {
    seen: Arc<Mutex<Vec<(String, String, String)>>>,
    tokens: Arc<Mutex<VecDeque<String>>>,
}

impl RecordingRds {
    #[must_use]
    pub(super) fn tokens(tokens: &[&str]) -> Self {
        Self {
            seen: Arc::default(),
            tokens: token_queue(tokens),
        }
    }

    pub(super) fn seen(&self) -> Vec<(String, String, String)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn mint_count(&self) -> usize {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl RdsTokenProvider for RecordingRds {
    fn mint(&self, endpoint: &RdsEndpoint<'_>) -> Result<SecretString, RdsCredentialsError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                endpoint.host.to_owned(),
                endpoint.port.to_owned(),
                endpoint.user.to_owned(),
            ));
        let mut tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        tokens
            .pop_front()
            .map(SecretString::from)
            .ok_or(RdsCredentialsError::NoCredentials)
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeAzureProvider {
    calls: Arc<AtomicUsize>,
    tokens: Arc<Mutex<VecDeque<String>>>,
    failing: Arc<AtomicBool>,
}

impl FakeAzureProvider {
    #[must_use]
    pub(super) fn tokens(tokens: &[&str]) -> Self {
        Self {
            calls: Arc::default(),
            tokens: token_queue(tokens),
            failing: Arc::default(),
        }
    }

    pub(super) fn fail(&self) {
        self.failing.store(true, Ordering::SeqCst);
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AzureTokenProvider for FakeAzureProvider {
    fn token(&self) -> Result<SecretString, AzureProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.load(Ordering::SeqCst) {
            return Err(AzureProviderError::Provider {
                message: "no Entra credential is available".to_owned(),
            });
        }
        let mut tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(SecretString::from(
            tokens
                .pop_front()
                .unwrap_or_else(|| AZURE_RAW_TOKEN.to_owned()),
        ))
    }
}

#[derive(Clone, Copy)]
pub(super) enum Build {
    Success,
    Failure,
}

#[derive(Clone)]
pub(super) struct FakeAzureFactory {
    builds: Arc<AtomicUsize>,
    scopes: Arc<Mutex<Vec<String>>>,
    outcomes: Arc<Mutex<VecDeque<Build>>>,
    provider: FakeAzureProvider,
}

impl FakeAzureFactory {
    #[must_use]
    pub(super) fn new(provider: FakeAzureProvider) -> Self {
        Self {
            builds: Arc::default(),
            scopes: Arc::default(),
            outcomes: Arc::default(),
            provider,
        }
    }

    pub(super) fn queue(&self, build: Build) {
        self.outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(build);
    }

    pub(super) fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    pub(super) fn scopes(&self) -> Vec<String> {
        self.scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AzureProviderFactory for FakeAzureFactory {
    fn build(&self, scope: &str) -> Result<Arc<dyn AzureTokenProvider>, AzureProviderError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        self.scopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scope.to_owned());
        let outcome = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match outcome {
            Some(Build::Failure) => Err(AzureProviderError::Provider {
                message: "no Entra credential is available".to_owned(),
            }),
            Some(Build::Success) | None => Ok(Arc::new(self.provider.clone())),
        }
    }
}

pub(super) fn azure_providers(tokens: &[&str]) -> TokenProviders {
    providers(
        RecordingRds::tokens(&["encoded"]),
        FakeAzureFactory::new(FakeAzureProvider::tokens(tokens)),
    )
}

pub(super) fn iam_enabled() -> TokenToggles {
    TokenToggles {
        iam_token_db_auth: true,
        azure_postgresql_auth: false,
    }
}

pub(super) fn azure_enabled() -> TokenToggles {
    TokenToggles {
        iam_token_db_auth: false,
        azure_postgresql_auth: true,
    }
}

fn prefixed(prefix: &[(&str, &str)], environment: &[(&str, &str)]) -> Vec<(&str, &str)> {
    let mut pairs = prefix.to_vec();
    pairs.extend_from_slice(environment);
    pairs
}

pub(super) fn writer_missing_message(environment: &[(&str, &str)]) -> String {
    let pairs = prefixed(&[("IAM_TOKEN_DB_AUTH", "true")], environment);
    let azure = azure_providers(&[AZURE_RAW_TOKEN]);
    build_writer_url(&settings(&pairs), iam_enabled(), false, &azure)
        .expect_err("missing writer fields must fail")
        .to_string()
}

pub(super) fn reader_missing_message(environment: &[(&str, &str)]) -> String {
    let prefix = [
        ("AZURE_POSTGRESQL_AUTH", "true"),
        ("DATABASE_HOST_READ_REPLICA", "reader.example"),
    ];
    let azure = azure_providers(&["azure-token"]);
    build_reader_url(
        &settings(&prefixed(&prefix, environment)),
        azure_enabled(),
        &azure,
    )
    .expect_err("unresolved reader fields must fail")
    .to_string()
}

pub(super) fn providers(rds: RecordingRds, factory: FakeAzureFactory) -> TokenProviders {
    TokenProviders {
        rds: Arc::new(rds),
        azure: Arc::new(AzureProviderCache::new(Arc::new(factory))),
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}").trim_matches('"').to_owned();
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, run);
    let entries = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, entries)
}
