use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::aws_settings::sources::Credentials as AwsCredentials;
use betterlite_bedrock::canvas_results;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::image_call::plan::CredentialSource;
use betterlite_bedrock::image_call::transport::{
    ClientFactory, ImageReply, ImageRequest, ImageTransport, TransferError,
};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::stability_edits;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use super::input::{Probe, Reply, ReplyFailure};

mod harness;

pub(super) use harness::{Harness, NOW_SECONDS};

#[derive(Clone, Default)]
pub(super) struct Seen(Arc<Mutex<Vec<ImageRequest>>>);

impl Seen {
    pub(super) fn requests(&self) -> Vec<ImageRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn image_reply(reply: &Reply) -> ImageReply {
    ImageReply {
        status: reply.status,
        headers: reply
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        body: reply.body.as_bytes().to_vec(),
    }
}

pub(crate) struct Client {
    reply: Reply,
    pub(super) seen: Seen,
}

impl Client {
    pub(super) fn new(reply: &Reply) -> Self {
        Self {
            reply: reply.clone(),
            seen: Seen::default(),
        }
    }
}

impl ImageTransport for Client {
    fn post(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransferError>> + Send {
        self.seen
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let reply = match self.reply.failure {
            None => Ok(image_reply(&self.reply)),
            Some(ReplyFailure::Timeout) => Err(TransferError::Timeout),
            Some(ReplyFailure::Failed) => Err(TransferError::Failed(TransportError::Closed)),
        };
        ready(reply)
    }
}

pub(crate) struct Factory {
    reply: Reply,
    calls: Mutex<Vec<(ClientKind, Option<Duration>)>>,
    pub(super) seen: Seen,
}

impl Factory {
    pub(super) fn new(reply: &Reply) -> Self {
        Self {
            reply: reply.clone(),
            calls: Mutex::new(Vec::new()),
            seen: Seen::default(),
        }
    }

    fn client(&self, kind: ClientKind, timeout: Option<Duration>) -> Client {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((kind, timeout));
        Client {
            reply: self.reply.clone(),
            seen: self.seen.clone(),
        }
    }

    pub(super) fn calls(&self) -> Vec<(ClientKind, Option<Duration>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientFactory for Factory {
    type Client = Client;

    fn sync_client(&self) -> Self::Client {
        self.client(ClientKind::Sync, None)
    }

    fn async_client(&self, timeout: Option<Duration>) -> Self::Client {
        self.client(ClientKind::Asynchronous, timeout)
    }
}

pub(crate) struct Source {
    credentials: AwsCredentials,
}

impl Source {
    pub(super) fn new(probe: &Probe) -> Self {
        Self {
            credentials: AwsCredentials {
                access_key_id: SecretString::from(probe.credentials.access_key_id.clone()),
                secret_access_key: SecretString::from(probe.credentials.secret_access_key.clone()),
                session_token: probe
                    .credentials
                    .session_token
                    .as_ref()
                    .map(|token| SecretString::from(token.clone())),
            },
        }
    }
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<AwsCredentials, CredentialError>> + Send {
        ready(Ok(self.credentials.clone()))
    }
}

// The typed catalog stays empty: the selector reads the raw model-cost map the case supplies.
pub(super) struct Catalog {
    typed: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Catalog {
    pub(super) fn new() -> Result<Self, CatalogError> {
        let typed = ModelCatalog::parse(b"{}", UnknownFields::Warn)?;
        let known = BTreeSet::new();
        let custom = BTreeSet::new();
        let environment = BTreeMap::new();
        Ok(Self {
            typed,
            known,
            custom,
            environment,
        })
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

// get_model_info reads the raw catalog entry; the same entry answers both family lookups.
pub(super) struct Prices<'cost>(pub(super) &'cost ParamMap);

impl canvas_results::ModelPriceLookup for Prices<'_> {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        let entry = self.0.get(model)?;
        let value = serde_json::to_value(entry).ok()?;
        serde_json::from_value(value).ok()
    }
}

impl stability_edits::reply::ModelPriceLookup for Prices<'_> {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        let entry = self.0.get(model)?;
        let value = serde_json::to_value(entry).ok()?;
        serde_json::from_value(value).ok()
    }
}

#[derive(Default)]
pub(super) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging_state() -> Result<LoggingState, super::Failure> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"stability.stable-image-inpaint-v1:0","call_type":"image_edit","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    Ok(LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated".to_owned(),
        Duration::default,
    )?)
}

pub(super) fn prepost_env<'env>(
    state: &LoggingState,
    redactor: &'env Redactor,
    sink: &'env DiscardingSink,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: state.start_time,
    }
}
