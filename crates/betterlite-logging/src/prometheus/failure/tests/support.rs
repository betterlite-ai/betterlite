use std::collections::{BTreeMap, BTreeSet};
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_error::normalize::class::Class;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::prometheus::failure::RequestedModelCatalog;
use crate::prometheus::failure::deployment_labels::{PayloadMetadataRecord, UserApiKeyFacts};
use crate::prometheus::failure::exception::LabelException;

const CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai"},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

pub(super) fn label_exception() -> LabelException {
    LabelException {
        class_name: String::new(),
        class: Class::Other,
        message: None,
        status_code: None,
        code: None,
        llm_provider: None,
        category: None,
        rate_limit_type: None,
        legacy_class_name: None,
    }
}

pub(super) fn with_message(mut exception: LabelException, message: &str) -> LabelException {
    exception.message = Some(message.to_owned());
    exception
}

pub(super) fn with_status(
    mut exception: LabelException,
    status_code: Option<&str>,
    code: Option<&str>,
) -> LabelException {
    exception.status_code = status_code.map(str::to_owned);
    exception.code = code.map(str::to_owned);
    exception
}

pub(super) fn with_class(
    mut exception: LabelException,
    class: Class,
    class_name: &str,
) -> LabelException {
    exception.class = class;
    exception.class_name = class_name.to_owned();
    exception
}

pub(super) fn with_provider(mut exception: LabelException, provider: &str) -> LabelException {
    exception.llm_provider = Some(provider.to_owned());
    exception
}

pub(super) fn with_legacy(mut exception: LabelException, name: &str) -> LabelException {
    exception.legacy_class_name = Some(name.to_owned());
    exception
}

pub(super) fn with_rate_limit(
    mut exception: LabelException,
    category: Option<&str>,
    rate_limit_type: Option<&str>,
) -> LabelException {
    exception.category = category.map(str::to_owned);
    exception.rate_limit_type = rate_limit_type.map(str::to_owned);
    exception
}

pub(super) fn metadata(pairs: &[(&str, &str)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| (String::from(*key), Json::Text(String::from(*value))))
        .collect()
}

pub(super) fn metadata_record() -> PayloadMetadataRecord {
    PayloadMetadataRecord {
        user_api_key_alias: None,
        user_api_key_user_email: None,
        user_api_key_team_id: None,
        user_api_key_team_alias: None,
        user_api_key_hash: None,
        requester_ip_address: None,
        user_agent: None,
    }
}

pub(super) fn api_key_facts() -> UserApiKeyFacts {
    UserApiKeyFacts {
        key_alias: None,
        user_email: None,
        team_id: None,
        team_alias: None,
        hashed_key: None,
        key_hash: None,
    }
}

#[derive(Clone, Default)]
pub(super) struct StubCatalog {
    pub(super) recognized: BTreeSet<String>,
    pub(super) team_public: BTreeSet<String>,
    pub(super) patterns: BTreeSet<String>,
}

impl RequestedModelCatalog for StubCatalog {
    fn is_recognized_model(&self, model: &str) -> bool {
        self.recognized.contains(model)
    }

    fn is_team_public_model(&self, model: &str) -> bool {
        self.team_public.contains(model)
    }

    fn matches_pattern_route(&self, model: &str) -> bool {
        self.patterns.contains(model)
    }
}

pub(super) struct ProviderFixture {
    pub(super) catalog: ModelCatalog,
    pub(super) known: BTreeSet<String>,
    pub(super) custom: BTreeSet<String>,
    pub(super) environment: BTreeMap<String, String>,
}

impl ProviderFixture {
    pub(super) fn new() -> Self {
        Self {
            catalog: ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
                .expect("the test catalog parses"),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are utf-8")
    }
}

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}
