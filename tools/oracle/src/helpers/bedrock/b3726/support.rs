use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::guardrail_hooks::StreamingSettings;
use betterlite_bedrock_guardrails::bedrock_transport::{BedrockGuardrail, BedrockGuardrailInit};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::bedrock_streaming::BedrockGuardrailStreamParams;
use betterlite_core::routes::CallType;
use betterlite_guardrails::bedrock_checks::request::ChecksEnvironment;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;
use secrecy::SecretString;

use super::GuardrailArgs;

const REGION: &str = "us-west-2";
const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const FIXED_NOW_SECONDS: u64 = 1_000_000;
const CHUNK_BUDGET_CHARS: u64 = 25_000;
// LiteLLM's BedrockGuardrail takes 0.5 for every check threshold when the config names none
const DEFAULT_THRESHOLD: Option<ConfidenceThreshold> = Some(ConfidenceThreshold(0.5_f64));
pub(super) const ENVIRONMENT: ChecksEnvironment<'static> = ChecksEnvironment {
    runtime_endpoint: None,
    region_name: None,
    region: None,
    bearer_token: None,
};

pub(super) struct Creds;

impl CredentialSource for Creds {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(Credentials {
            access_key_id: SecretString::from(ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
            session_token: None,
        }))
    }
}

pub(super) fn helper_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS))
        .unwrap_or(UNIX_EPOCH)
}

fn event_text(event: GuardrailEventHooks) -> String {
    let text = match event {
        GuardrailEventHooks::PreCall => "pre_call",
        GuardrailEventHooks::PostCall => "post_call",
        GuardrailEventHooks::DuringCall => "during_call",
        GuardrailEventHooks::LoggingOnly => "logging_only",
        GuardrailEventHooks::PreMcpCall => "pre_mcp_call",
        GuardrailEventHooks::DuringMcpCall => "during_mcp_call",
        GuardrailEventHooks::PostMcpCall => "post_mcp_call",
        GuardrailEventHooks::RealtimeInputTranscription => "realtime_input_transcription",
    };
    text.to_owned()
}

pub(super) fn build_guardrail(args: &GuardrailArgs) -> Result<BedrockGuardrail, Box<HelperError>> {
    let mut params = BTreeMap::new();
    for (key, value) in [
        ("aws_access_key_id", ACCESS_KEY),
        ("aws_secret_access_key", SECRET_KEY),
        ("aws_region_name", REGION),
    ] {
        params.insert(key.to_owned(), GuardrailValue::Text(value.to_owned()));
    }
    BedrockGuardrail::__init__(BedrockGuardrailInit {
        guardrail_identifier: Some(args.identifier.clone()),
        guardrail_version: Some(args.version.clone()),
        disable_exception_on_block: args.disable_exception_on_block,
        checks: None,
        content_filter_threshold: DEFAULT_THRESHOLD,
        prompt_attack_threshold: DEFAULT_THRESHOLD,
        pii_confidence_threshold: DEFAULT_THRESHOLD,
        chunk_budget_chars: CHUNK_BUDGET_CHARS,
        contextual_grounding: false,
        latest_role_message_only: args.latest_role_message_only.unwrap_or(false),
        mask_request_content: false,
        mask_response_content: false,
        buffer_until_moderated: args.buffer_until_moderated,
        streaming_sampling_rate: None,
        streaming_end_of_stream_only: args.streaming_end_of_stream_only,
        buffer_release_on_scan: args.buffer_release_on_scan,
        optional_params: OpaqueParams(params),
        timeout: None,
    })
    .or_config_error()
}

pub(super) fn base_settings(args: &GuardrailArgs) -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: Some(args.name.clone()),
            supported_event_hooks: None,
            event_hook: args
                .event_hook
                .map(|event| LitellmParamsMode::One(event_text(event))),
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

pub(super) fn streaming_settings(
    args: &GuardrailArgs,
    guardrail: &BedrockGuardrail,
    keepalive_interval_seconds: Option<f64>,
) -> StreamingSettings {
    StreamingSettings {
        guardrail_name: args.name.clone(),
        params: BedrockGuardrailStreamParams {
            buffer_until_moderated: guardrail.buffer_until_moderated,
            streaming_sampling_rate: guardrail.streaming_sampling_rate,
            streaming_end_of_stream_only: guardrail.streaming_end_of_stream_only,
            buffer_release_on_scan: guardrail.buffer_release_on_scan,
        },
        mask_response_content: guardrail.mask_response_content,
        keepalive_interval_seconds,
    }
}

pub(super) struct Routes(BTreeMap<String, Vec<CallType>>);

impl Routes {
    // LiteLLM's API_ROUTE_TO_CALL_TYPES for the routes the cases drive
    pub(super) fn new() -> Self {
        Self(BTreeMap::from([
            (
                "/v1/chat/completions".to_owned(),
                vec![CallType::Acompletion, CallType::Completion],
            ),
            (
                "/v1/responses".to_owned(),
                vec![CallType::Aresponses, CallType::Responses],
            ),
        ]))
    }
}

impl RouteCallTypes<CallType> for Routes {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<CallType>> {
        self.0.get(route).cloned()
    }
}
