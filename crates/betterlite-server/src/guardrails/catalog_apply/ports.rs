use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::routes::ApplyGuardrailResponse;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_guardrails::guardrail_registry::callback::GuardrailCallback;
use serde::Serialize;
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;
use crate::guardrails::callback_caps::errors::{HttpException, ProxyHookError};

use super::error::CatalogApplyError;

pub type Body = BTreeMap<String, GuardrailValue>;

// LiteLLM reports this route to the logging callbacks as the pass-through endpoint's call type
pub(super) const APPLY_CALL_TYPE: &str = "apply_guardrail";

#[derive(Debug, Error)]
pub enum ApplyLogError {
    #[error("the guardrail's logging handle is not accepting calls")]
    Unavailable,
    #[error("{message}")]
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuccessPayload {
    pub response: ApplyGuardrailResponse,
}

pub struct PreCallOutcome {
    pub data: Body,
    pub log: Option<Arc<dyn ApplyLog>>,
}

// LiteLLM's common_processing_pre_call_logic and the proxy logging object's two hook methods
#[async_trait]
pub trait ApplyHooks: Send + Sync {
    async fn pre_call(
        &self,
        data: Body,
        caller: &AuthContext,
        call_type: &str,
    ) -> Result<PreCallOutcome, ProxyHookError>;

    async fn post_call_failure(
        &self,
        caller: &AuthContext,
        error: &CatalogApplyError,
        data: &Body,
    ) -> Result<Option<HttpException>, ProxyHookError>;

    async fn post_call_success(
        &self,
        data: &Body,
        caller: &AuthContext,
        response: ApplyGuardrailResponse,
    ) -> Result<Option<ApplyGuardrailResponse>, ProxyHookError>;
}

// The LiteLLM logging object's update_messages and its success and failure handlers
#[async_trait]
pub trait ApplyLog: Send + Sync {
    fn mark_pass_through(&self, messages: Vec<OpaqueParams>) -> Result<(), ApplyLogError>;

    async fn async_failure_handler(
        &self,
        error: &CatalogApplyError,
        traceback: &str,
    ) -> Result<(), ApplyLogError>;

    fn submit_failure_handler(
        &self,
        error: &CatalogApplyError,
        traceback: &str,
    ) -> Result<(), ApplyLogError>;

    async fn async_success_handler(
        &self,
        payload: SuccessPayload,
        start_time: SystemTime,
        end_time: SystemTime,
        cache_hit: bool,
    ) -> Result<(), ApplyLogError>;

    fn submit_success_handler(
        &self,
        payload: SuccessPayload,
        start_time: SystemTime,
        end_time: SystemTime,
        cache_hit: bool,
    ) -> Result<(), ApplyLogError>;
}

// The class dispatch B8666 builds turns a held callback into a run guardrail
#[async_trait]
pub trait GuardrailRunner: Send + Sync {
    async fn apply_guardrail(
        &self,
        callback: &GuardrailCallback,
        inputs: GuardrailApiInputs,
        request_data: &mut Body,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailApiInputs, GuardrailFailure>;
}

// a mount that has no proxy logging object attached yet: every hook passes the call through
pub struct NoHooks;

#[async_trait]
impl ApplyHooks for NoHooks {
    async fn pre_call(
        &self,
        data: Body,
        _caller: &AuthContext,
        _call_type: &str,
    ) -> Result<PreCallOutcome, ProxyHookError> {
        Ok(PreCallOutcome { data, log: None })
    }

    async fn post_call_failure(
        &self,
        _caller: &AuthContext,
        _error: &CatalogApplyError,
        _data: &Body,
    ) -> Result<Option<HttpException>, ProxyHookError> {
        Ok(None)
    }

    async fn post_call_success(
        &self,
        _data: &Body,
        _caller: &AuthContext,
        _response: ApplyGuardrailResponse,
    ) -> Result<Option<ApplyGuardrailResponse>, ProxyHookError> {
        Ok(None)
    }
}

pub struct NoSpans;

impl GuardrailSpanSink for NoSpans {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

pub struct ApplyPorts {
    pub runner: Arc<dyn GuardrailRunner>,
    pub hooks: Arc<dyn ApplyHooks>,
    pub sink: Arc<dyn GuardrailSpanSink + Send + Sync>,
}
