use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::decision_check::GuardrailChecks;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::BedrockGuardrailConfigModel;
use betterlite_config::sections::guardrail_translation::bedrock_streaming::SamplingRate;

use super::error::InitializerError;
use super::extras;

mod tests;

const SAMPLING_RATE: &str = "streaming_sampling_rate";
const CHUNK_BUDGET: &str = "chunk_budget_chars";
// LiteLLM's BedrockGuardrailConfigModel declares 0.5 for all three thresholds
const DEFAULT_THRESHOLD: Option<ConfidenceThreshold> = Some(ConfidenceThreshold(0.5));

#[derive(Debug, Clone)]
pub struct BedrockArgs {
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub mask_request_content: GuardrailFlag,
    pub mask_response_content: GuardrailFlag,
    pub only_scan_new_messages: GuardrailFlag,
    pub latest_role_message_only: GuardrailFlag,
    pub timeout: Option<f64>,
    pub config: BedrockGuardrailConfigModel,
    pub streaming: BedrockStreamingParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BedrockStreamingParams {
    pub buffer_until_moderated: GuardrailFlag,
    pub streaming_sampling_rate: SamplingRate,
    pub streaming_end_of_stream_only: GuardrailFlag,
    pub buffer_release_on_scan: GuardrailFlag,
}

impl BedrockStreamingParams {
    pub fn from_extras(extra: &OpaqueParams) -> Result<Self, InitializerError> {
        Ok(Self {
            buffer_until_moderated: extras::flag(extra, "streaming_buffer_until_moderated", true)
                .map_err(streaming_param)?,
            streaming_sampling_rate: streaming_rate(extra)?,
            streaming_end_of_stream_only: extras::flag(
                extra,
                "streaming_end_of_stream_only",
                false,
            )
            .map_err(streaming_param)?,
            buffer_release_on_scan: extras::flag(extra, "streaming_buffer_release_on_scan", false)
                .map_err(streaming_param)?,
        })
    }
}

pub fn initialize_bedrock(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<BedrockArgs, InitializerError> {
    let streaming = BedrockStreamingParams::from_extras(&params.base.extra)?;
    let mut config = bedrock_config(&params.base.extra)?;
    if let Some(GuardrailChecks::Bedrock(checks)) = &params.checks {
        config.checks = Some(checks.clone());
    }
    Ok(BedrockArgs {
        name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        mask_request_content: params.base.fields.mask_request_content.unwrap_or_default(),
        mask_response_content: params.base.fields.mask_response_content.unwrap_or_default(),
        only_scan_new_messages: params.base.scan_flags.only_scan_new_messages,
        latest_role_message_only: params.base.scan_flags.latest_role_message_only,
        timeout: params.base.fields.timeout,
        config,
        streaming,
    })
}

fn bedrock_config(extra: &OpaqueParams) -> Result<BedrockGuardrailConfigModel, InitializerError> {
    let mut config = BedrockGuardrailConfigModel::default();
    config.guardrail_identifier = extras::read(extra, "guardrailIdentifier", None)?;
    config.guardrail_version = extras::read(extra, "guardrailVersion", None)?;
    config.disable_exception_on_block = extras::read(
        extra,
        "disable_exception_on_block",
        config.disable_exception_on_block,
    )?;
    config.content_filter_threshold =
        extras::read(extra, "content_filter_threshold", DEFAULT_THRESHOLD)?;
    config.prompt_attack_threshold =
        extras::read(extra, "prompt_attack_threshold", DEFAULT_THRESHOLD)?;
    config.pii_confidence_threshold =
        extras::read(extra, "pii_confidence_threshold", DEFAULT_THRESHOLD)?;
    config.chunk_budget_chars =
        extras::read(extra, CHUNK_BUDGET, None)?.unwrap_or(config.chunk_budget_chars);
    config.contextual_grounding = extras::flag(
        extra,
        "contextual_grounding_from_messages",
        config.contextual_grounding,
    )?;
    config.aws_region_name = extras::read(extra, "aws_region_name", None)?;
    config.aws_access_key_id = extras::read(extra, "aws_access_key_id", None)?;
    config.aws_secret_access_key = extras::read(extra, "aws_secret_access_key", None)?;
    config.aws_session_token = extras::read(extra, "aws_session_token", None)?;
    config.aws_session_name = extras::read(extra, "aws_session_name", None)?;
    config.aws_profile_name = extras::read(extra, "aws_profile_name", None)?;
    config.aws_role_name = extras::read(extra, "aws_role_name", None)?;
    config.aws_web_identity_token = extras::read(extra, "aws_web_identity_token", None)?;
    config.aws_sts_endpoint = extras::read(extra, "aws_sts_endpoint", None)?;
    config.aws_external_id = extras::read(extra, "aws_external_id", None)?;
    config.aws_bedrock_runtime_endpoint =
        extras::read(extra, "aws_bedrock_runtime_endpoint", None)?;
    Ok(config)
}

fn streaming_param(error: InitializerError) -> InitializerError {
    match error {
        InitializerError::InvalidParam(name) => InitializerError::InvalidStreamingParam { name },
        other @ (InitializerError::InvalidStreamingParam { .. }
        | InitializerError::MissingEnterprisePackage
        | InitializerError::MissingApiKey
        | InitializerError::MissingProfileName) => other,
    }
}

fn streaming_rate(extra: &OpaqueParams) -> Result<SamplingRate, InitializerError> {
    let rate = extras::read::<u64>(extra, SAMPLING_RATE, None)
        .map_err(streaming_param)?
        .unwrap_or(5);
    SamplingRate::try_from(rate).map_err(|_below_one| InitializerError::InvalidStreamingParam {
        name: SAMPLING_RATE.to_owned(),
    })
}
