use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::rules::{RuleDecision, RuleEvaluator};
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::registry::error::RegistryError;
use betterlite_logging::worker::Admission;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::correlation::CallOutcome;
use betterlite_sdk::call_setup::credentials::{Credential, CredentialLookup};
use betterlite_sdk::call_wrapper::cache::{CacheHandler, CacheMode, CacheOutcome};
use betterlite_sdk::call_wrapper::error::CallError;
use betterlite_sdk::call_wrapper::logging::{AsyncSuccessLog, CallLogging, SuccessLogScheduler};
use betterlite_sdk::call_wrapper::max_tokens::MaxTokensModifier;
use betterlite_sdk::call_wrapper::metadata::ResponseMetadataUpdater;
use betterlite_sdk::call_wrapper::ports::{CallTimes, IdSource, WrapperClock};
use betterlite_sdk::call_wrapper::state::{KnownCallback, LoggingHandle};
use betterlite_sdk::call_wrapper::stream::StreamAssembly;
use chrono::{DateTime, FixedOffset};

// The cases configure no rules, so every check stops at the first index
pub(super) struct NoRules;

impl RuleEvaluator for NoRules {
    fn evaluate(&self, _index: usize, _input: &str) -> Option<RuleDecision> {
        None
    }
}

// litellm's callback lists are empty in every case, so the logging object's handlers do nothing
pub(super) struct NoLogging;

#[async_trait]
impl CallLogging for NoLogging {
    fn success(&self, _handle: &LoggingHandle, _result: SuccessResult, _times: CallTimes) {}

    async fn async_success(
        &self,
        _handle: &LoggingHandle,
        _result: SuccessResult,
        _times: CallTimes,
    ) {
    }

    // ast-grep-ignore: no-long-names
    fn sync_success_callbacks_for_async_calls(
        &self,
        _handle: &LoggingHandle,
        _result: SuccessResult,
        _times: CallTimes,
    ) {
    }

    fn failure_sync(&self, _handle: &LoggingHandle, _error: &CallError, _times: CallTimes) {}

    async fn failure_async(&self, _handle: &LoggingHandle, _error: &CallError, _times: CallTimes) {}
}

// The cases have no logger names, so a resolver is only present to satisfy the setup state
pub(super) struct NoResolver;

impl LoggerResolver for NoResolver {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        Err(RegistryError::UnavailableAdapter {
            name: name.to_owned(),
        })
    }
}

pub(super) struct NoKnown;

impl KnownCallback for NoKnown {
    fn is_known(&self, _name: &str) -> bool {
        false
    }
}

pub(super) struct NoCredentials;

impl CredentialLookup for NoCredentials {
    fn find_credential(&self, _credential_name: &str) -> Option<&Credential> {
        None
    }
}

pub(super) struct NoMaxTokens;

impl MaxTokensModifier for NoMaxTokens {
    fn modified_max_tokens(
        &self,
        _model: &str,
        _hf_model_name: Option<&str>,
        _messages: Option<&Json>,
        _user_max_tokens: i64,
    ) -> Option<i64> {
        None
    }
}

pub(super) struct NoAssembly;

impl StreamAssembly for NoAssembly {
    fn assemble(
        &self,
        _outcome: &CallOutcome,
        _messages: Option<&Json>,
        _logging: Option<&LoggingHandle>,
    ) -> Result<Option<Json>, CallError> {
        Ok(None)
    }
}

pub(super) struct NoMetadata;

impl ResponseMetadataUpdater for NoMetadata {
    fn update(
        &self,
        _outcome: &CallOutcome,
        _logging: &LoggingHandle,
        _model: Option<&str>,
        _request: &RequestMap,
        _times: CallTimes,
    ) {
    }
}

// The cases read nothing off the logging object, so the background job is dropped
pub(super) struct DropScheduler;

impl SuccessLogScheduler for DropScheduler {
    fn schedule(&self, _job: AsyncSuccessLog) -> Admission {
        Admission::Accepted
    }
}

pub(super) struct FixedClock;

impl WrapperClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::UNIX_EPOCH.fixed_offset()
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }
}

// The cases only report whether a call id exists, never its value
pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn next_call_id(&self) -> String {
        "generated-1".to_owned()
    }
}

// litellm.cache in the cases: the response it already holds, if the case seeds one
pub(super) struct ScriptedCache {
    pub(super) hit: Option<Json>,
}

#[async_trait]
impl CacheHandler for ScriptedCache {
    async fn lookup(
        &self,
        _request: &RequestMap,
        _call_type: &str,
        _mode: CacheMode,
    ) -> Result<Option<CacheOutcome>, CallError> {
        Ok(Some(CacheOutcome {
            cached: self.hit.clone().map(CallOutcome::Response),
            final_embedding: None,
            all_embeddings_cached: false,
            uncached_input: None,
            preset_cache_key: None,
        }))
    }

    async fn store(
        &self,
        _request: &RequestMap,
        _outcome: &CallOutcome,
        _call_type: &str,
        _mode: CacheMode,
    ) -> Result<(), CallError> {
        Ok(())
    }

    fn wrap_streaming_result(&self, outcome: CallOutcome, _call_type: &str) -> CallOutcome {
        outcome
    }

    fn combine_cached_embedding(
        &self,
        _final_embedding: &Json,
        fresh: CallOutcome,
        _times: CallTimes,
    ) -> CallOutcome {
        fresh
    }
}
