use std::time::SystemTime;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;
use serde::Deserialize;

use crate::crowdstrike_aidr::error::CrowdStrikeError;
use crate::crowdstrike_streaming::{StreamingParams, StreamingSettings, read_streaming_params};
use crate::crowdstrike_writeback::SkipFlags;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_records::record::GuardrailSpanSink;

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];

const API_KEY_ENV: &str = "CS_AIDR_TOKEN";
const API_BASE_ENV: &str = "CS_AIDR_BASE_URL";

#[derive(Debug, Clone)]
pub struct Settings {
    pub guardrail_name: Option<String>,
    pub api_key: SecretString,
    pub api_base: String,
    pub fail_on_error: bool,
    pub streaming: StreamingSettings,
    pub timeout: Option<f64>,
    pub skip: SkipFlags,
}

// What LiteLLM's CrowdStrikeAIDRHandler.__init__ takes before it reads the environment
#[derive(Clone, Default)]
pub struct ConstructorArgs {
    pub guardrail_name: Option<String>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub fail_on_error: Option<bool>,
    pub streaming: StreamingParams,
    pub timeout: Option<f64>,
}

pub struct Dependencies {
    pub client: reqwest::Client,
    pub sink: Box<dyn GuardrailSpanSink + Send + Sync>,
    pub now: fn() -> SystemTime,
}

pub struct CrowdStrikeAidr {
    pub settings: Settings,
    pub base: GuardrailBase,
    pub client: reqwest::Client,
    pub sink: Box<dyn GuardrailSpanSink + Send + Sync>,
    pub now: fn() -> SystemTime,
}

impl Settings {
    pub fn resolve<L: EnvironmentLookup>(
        args: &ConstructorArgs,
        lookup: &L,
    ) -> Result<Self, CrowdStrikeError> {
        let api_key = args
            .api_key
            .clone()
            .filter(|key| !key.is_empty())
            .or_else(|| lookup.lookup(API_KEY_ENV))
            .filter(|key| !key.is_empty())
            .ok_or(CrowdStrikeError::MissingEnvironment { name: API_KEY_ENV })?;
        let api_base = args
            .api_base
            .clone()
            .filter(|base| !base.is_empty())
            .or_else(|| lookup.lookup(API_BASE_ENV))
            .filter(|base| !base.is_empty())
            .ok_or(CrowdStrikeError::MissingEnvironment { name: API_BASE_ENV })?;
        Ok(Self {
            guardrail_name: args.guardrail_name.clone(),
            api_key: SecretString::from(api_key),
            api_base,
            fail_on_error: args.fail_on_error.unwrap_or(true),
            streaming: StreamingSettings::from_params(args.streaming),
            timeout: args.timeout,
            skip: SkipFlags::default(),
        })
    }
}

impl CrowdStrikeAidr {
    pub fn new<L: EnvironmentLookup>(
        args: &ConstructorArgs,
        lookup: &L,
        dependencies: Dependencies,
    ) -> Result<Self, CrowdStrikeError> {
        let settings = Settings::resolve(args, lookup)?;
        let base = GuardrailBase {
            settings: GuardrailSettings {
                guardrail_name: settings.guardrail_name.clone(),
                supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
                event_hook: None,
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
                timeout: settings.timeout,
                switches: GuardrailSwitches::default(),
            },
            mode: None,
        };
        Ok(Self {
            settings,
            base,
            client: dependencies.client,
            sink: dependencies.sink,
            now: dependencies.now,
        })
    }

    // LiteLLM copies the params onto the handler, then reloads the streaming settings from them
    pub fn update_in_memory_params(
        &mut self,
        params: &LitellmParams,
    ) -> Result<(), CrowdStrikeError> {
        self.base.update_params(params);
        if let Some(key) = &params.base.fields.api_key {
            self.settings.api_key = key.clone();
        }
        if let Some(base) = &params.base.fields.api_base {
            self.settings.api_base.clone_from(base);
        }
        if let Some(GuardrailValue::Boolean(flag)) = params.base.extra.0.get("fail_on_error") {
            self.settings.fail_on_error = *flag;
        }
        self.settings.streaming = StreamingSettings::from_params(read_streaming_params(params)?);
        Ok(())
    }
}

// LiteLLM's CrowdStrikeAIDRGuardrailConfigModel, shortened to the identifier limit
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CrowdStrikeAidrConfig {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub fail_on_error: Option<bool>,
    #[serde(rename = "streaming_buffer_until_moderated")]
    pub buffer_until_moderated: Option<bool>,
    #[serde(rename = "streaming_buffer_release_on_scan")]
    pub buffer_release_on_scan: Option<bool>,
    #[serde(rename = "streaming_end_of_stream_only")]
    pub end_of_stream_only: Option<bool>,
    #[serde(rename = "streaming_sampling_rate")]
    pub sampling_rate: Option<u64>,
}

impl GuardrailConfigModelIdentity for CrowdStrikeAidrConfig {
    fn ui_friendly_name() -> &'static str {
        "CrowdStrike AIDR Guardrail"
    }
}
