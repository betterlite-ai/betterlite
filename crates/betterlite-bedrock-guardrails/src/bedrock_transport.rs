use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::bedrock_streaming::{
    BedrockConfigError, BedrockGuardrailStreamParams, SamplingRate,
};

use crate::constants::GUARDRAIL_NAME;
use crate::guardrail_checks::{ChecksInput, normalize_checks};

#[path = "bedrock_transport/apply_guardrail_path/tests.rs"]
mod apply_guardrail_path;
#[path = "bedrock_transport/bearer_credentials/tests.rs"]
mod bearer_credentials;
#[path = "bedrock_transport/bearer_request/tests.rs"]
mod bearer_request;
#[path = "bedrock_transport/checks_path/tests.rs"]
mod checks_path;
#[path = "bedrock_transport/credential_fetch/tests.rs"]
mod credential_fetch;
#[path = "bedrock_transport/endpoint_resolution/tests.rs"]
mod endpoint_resolution;
#[path = "bedrock_transport/error_parsing/tests.rs"]
mod error_parsing;
#[path = "bedrock_transport/exception_handling/tests.rs"]
mod exception_handling;
#[path = "bedrock_transport/init/tests.rs"]
mod init;
#[path = "bedrock_transport/normalize_checks/tests.rs"]
mod normalize_checks;
#[path = "bedrock_transport/normalize_checks_error/tests.rs"]
mod normalize_checks_error;
#[path = "bedrock_transport/post_call/tests.rs"]
mod post_call;
#[path = "bedrock_transport/proxied_transport/tests.rs"]
mod proxied_transport;
#[path = "bedrock_transport/region_resolution/tests.rs"]
mod region_resolution;
#[path = "bedrock_transport/sigv4_override/tests.rs"]
mod sigv4_override;
#[path = "bedrock_transport/tests/support.rs"]
mod test_support;
#[path = "bedrock_transport/transport_logging/tests.rs"]
mod transport_logging;

#[derive(Debug, thiserror::Error)]
pub enum BedrockGuardrailError {
    #[error(
        "Bedrock guardrail accepts either 'guardrailIdentifier'/'guardrailVersion' (ApplyGuardrail) or 'checks' (InvokeGuardrailChecks), not both."
    )]
    BothApiPaths,
    #[error(
        "BedrockGuardrail: 'checks' block contained only unrecognized or empty keys {keys:?}. Known keys: [\"contentFilter\", \"promptAttack\", \"sensitiveInformation\"]. Fix the guardrail config or remove the 'checks' block to use ApplyGuardrail mode."
    )]
    NoUsableChecks { keys: Vec<String> },
    #[error("the Bedrock streaming parameters do not decode")]
    Streaming(#[from] BedrockConfigError),
    #[error("the Bedrock checks cannot be encoded")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct BedrockGuardrailInit {
    pub guardrail_identifier: Option<String>,
    pub guardrail_version: Option<String>,
    pub disable_exception_on_block: Option<GuardrailFlag>,
    pub checks: Option<ChecksInput>,
    pub content_filter_threshold: Option<ConfidenceThreshold>,
    pub prompt_attack_threshold: Option<ConfidenceThreshold>,
    pub pii_confidence_threshold: Option<ConfidenceThreshold>,
    pub chunk_budget_chars: u64,
    pub contextual_grounding: GuardrailFlag,
    pub latest_role_message_only: GuardrailFlag,
    pub mask_request_content: GuardrailFlag,
    pub mask_response_content: GuardrailFlag,
    pub buffer_until_moderated: Option<GuardrailFlag>,
    pub streaming_sampling_rate: Option<SamplingRate>,
    pub streaming_end_of_stream_only: Option<GuardrailFlag>,
    pub buffer_release_on_scan: Option<GuardrailFlag>,
    pub optional_params: OpaqueParams,
    pub timeout: Option<Duration>,
}

// LiteLLM's class BedrockGuardrail(CustomGuardrail, BaseAWSLLM) inherits both bases; Rust composes the guardrail base and the AWS settings instead
#[derive(Debug, Clone)]
pub struct BedrockGuardrail {
    pub guardrail_identifier: Option<String>,
    pub guardrail_version: Option<String>,
    pub guardrail_provider: &'static str,
    pub chunk_budget_chars: u64,
    pub contextual_grounding: GuardrailFlag,
    pub latest_role_message_only: GuardrailFlag,
    pub checks: Option<BTreeMap<String, GuardrailValue>>,
    pub content_filter_threshold: Option<ConfidenceThreshold>,
    pub prompt_attack_threshold: Option<ConfidenceThreshold>,
    pub pii_confidence_threshold: Option<ConfidenceThreshold>,
    pub optional_params: OpaqueParams,
    pub disable_exception_on_block: GuardrailFlag,
    pub mask_request_content: GuardrailFlag,
    pub mask_response_content: GuardrailFlag,
    pub supported_event_hooks: Vec<GuardrailEventHooks>,
    pub buffer_until_moderated: GuardrailFlag,
    pub streaming_sampling_rate: SamplingRate,
    pub streaming_end_of_stream_only: GuardrailFlag,
    pub buffer_release_on_scan: GuardrailFlag,
    pub use_native_during_call_hook: GuardrailFlag,
    pub timeout: Option<Duration>,
}

impl BedrockGuardrail {
    pub fn __init__(mut init: BedrockGuardrailInit) -> Result<Self, BedrockGuardrailError> {
        let streaming = streaming_params(&init)?;
        let checks = normalize_checks(init.checks.take())?;
        if checks.is_some()
            && (init.guardrail_identifier.is_some() || init.guardrail_version.is_some())
        {
            return Err(BedrockGuardrailError::BothApiPaths);
        }
        let mut guardrail = Self::build(init, checks);
        guardrail.set_streaming_params(streaming);
        warn_about_masking(&guardrail);
        log_initialized(&guardrail);
        Ok(guardrail)
    }

    #[must_use]
    fn build(init: BedrockGuardrailInit, checks: Option<BTreeMap<String, GuardrailValue>>) -> Self {
        let defaults = BedrockGuardrailStreamParams::default();
        Self {
            guardrail_identifier: init.guardrail_identifier,
            guardrail_version: init.guardrail_version,
            guardrail_provider: GUARDRAIL_NAME,
            chunk_budget_chars: init.chunk_budget_chars,
            contextual_grounding: init.contextual_grounding,
            latest_role_message_only: init.latest_role_message_only,
            checks,
            content_filter_threshold: init.content_filter_threshold,
            prompt_attack_threshold: init.prompt_attack_threshold,
            pii_confidence_threshold: init.pii_confidence_threshold,
            optional_params: init.optional_params,
            disable_exception_on_block: init.disable_exception_on_block.unwrap_or(false),
            mask_request_content: init.mask_request_content,
            mask_response_content: init.mask_response_content,
            supported_event_hooks: Self::get_supported_event_hooks(),
            buffer_until_moderated: defaults.buffer_until_moderated,
            streaming_sampling_rate: defaults.streaming_sampling_rate,
            streaming_end_of_stream_only: defaults.streaming_end_of_stream_only,
            buffer_release_on_scan: defaults.buffer_release_on_scan,
            use_native_during_call_hook: true,
            timeout: init.timeout,
        }
    }

    fn set_streaming_params(&mut self, streaming: BedrockGuardrailStreamParams) {
        self.buffer_until_moderated = streaming.buffer_until_moderated;
        self.streaming_sampling_rate = streaming.streaming_sampling_rate;
        self.streaming_end_of_stream_only = streaming.streaming_end_of_stream_only;
        self.buffer_release_on_scan = streaming.buffer_release_on_scan;
    }

    // LiteLLM's list is GuardrailEventHooks::pre_call, GuardrailEventHooks::post_call and GuardrailEventHooks::during_call, then the two MCP hooks
    #[must_use]
    pub fn get_supported_event_hooks() -> Vec<GuardrailEventHooks> {
        vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PreMcpCall,
            GuardrailEventHooks::DuringMcpCall,
        ]
    }
}

fn streaming_params(
    init: &BedrockGuardrailInit,
) -> Result<BedrockGuardrailStreamParams, BedrockGuardrailError> {
    let mut extras = BTreeMap::new();
    insert_streaming_value(
        &mut extras,
        "streaming_buffer_until_moderated",
        init.buffer_until_moderated.map(GuardrailValue::Boolean),
    );
    insert_streaming_value(
        &mut extras,
        "streaming_sampling_rate",
        init.streaming_sampling_rate
            .map(|rate| GuardrailValue::UnsignedInteger(rate.0)),
    );
    insert_streaming_value(
        &mut extras,
        "streaming_end_of_stream_only",
        init.streaming_end_of_stream_only
            .map(GuardrailValue::Boolean),
    );
    insert_streaming_value(
        &mut extras,
        "streaming_buffer_release_on_scan",
        init.buffer_release_on_scan.map(GuardrailValue::Boolean),
    );
    BedrockGuardrailStreamParams::from_extras(Some(&extras)).map_err(BedrockGuardrailError::from)
}

fn insert_streaming_value(
    extras: &mut BTreeMap<String, GuardrailValue>,
    key: &str,
    value: Option<GuardrailValue>,
) {
    if let Some(value) = value {
        extras.insert(key.to_owned(), value);
    }
}

fn warn_about_masking(guardrail: &BedrockGuardrail) {
    if guardrail.checks.is_some()
        && (guardrail.mask_request_content || guardrail.mask_response_content)
    {
        tracing::warn!(
            "Bedrock Guardrail: mask_request_content and mask_response_content have no effect with checks, since InvokeGuardrailChecks is detect-only"
        );
    }
}

fn log_initialized(guardrail: &BedrockGuardrail) {
    let checks = guardrail
        .checks
        .as_ref()
        .map(|checks| checks.keys().cloned().collect::<Vec<String>>());
    tracing::debug!(
        guardrail_identifier = ?guardrail.guardrail_identifier,
        guardrail_version = ?guardrail.guardrail_version,
        ?checks,
        "Bedrock Guardrail initialized"
    );
}
