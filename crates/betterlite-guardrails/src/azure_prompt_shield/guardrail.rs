use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use super::billing::BillingUsage;
use super::cost::{COST_TIER_KEY, CostTier, PRICE_KEY, resolve_cost_tier, resolve_price};
use super::error::{PromptShieldError, describe_value};
use super::secret::resolved_reference;
use super::wire::{PromptShieldRequest, PromptShieldResponse};
use crate::azure_content_safety::api_version::resolve_api_version;
use crate::azure_content_safety::base::{AzureGuardrailBase, DEFAULT_TIMEOUT};
use crate::azure_content_safety::constants::MAX_TEXT_LENGTH;
use crate::azure_content_safety::text_splitting::split_text_by_words;
use crate::azure_content_safety::user_prompt::get_user_prompt_from_request;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_base::settings::GuardrailSettings;
use crate::hook_initializers::secret_lookup::SecretLookup;

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
];

// LiteLLM's _RESPONSES_API_CALL_TYPES, whose requests may carry no messages field at all.
const RESPONSES_CALL_TYPES: [&str; 2] = ["responses", "aresponses"];
const ATTACK_PATH: &str = "text:shieldPrompt";
const API_VERSION_KEY: &str = "api_version";

#[derive(Debug, Clone)]
pub struct PromptShieldParams {
    pub api_key: SecretString,
    pub api_base: String,
    pub api_version: Option<String>,
    pub timeout: Option<Duration>,
    pub cost_tier: Option<GuardrailValue>,
    pub price_per_1000_text_records: Option<GuardrailValue>,
    pub settings: GuardrailSettings,
    pub strict_modes: bool,
    pub async_handler: Client,
}

#[derive(Debug)]
pub struct AzurePromptShieldGuardrail {
    pub base: GuardrailBase,
    pub transport: AzureGuardrailBase,
    pub cost_tier: Option<CostTier>,
    pub price_per_1000_text_records: Option<Decimal>,
}

// What LiteLLM's pre-call hook hands back: the unchanged data dict, or None.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreCallReturn {
    Data,
    None,
}

impl AzurePromptShieldGuardrail {
    pub fn new(
        params: PromptShieldParams,
        lookup: &dyn SecretLookup,
    ) -> Result<Self, PromptShieldError> {
        let cost_tier = resolve_cost_tier(params.cost_tier.as_ref(), lookup)?;
        let price_per_1000_text_records = resolve_price(
            params.price_per_1000_text_records.as_ref(),
            cost_tier,
            lookup,
        )?;
        let mut settings = params.settings;
        if settings.supported_event_hooks.is_none() {
            settings.supported_event_hooks = Some(SUPPORTED_EVENT_HOOKS.to_vec());
        }
        settings.switches.use_native_lifecycle_hooks = true;
        settings.switches.uses_apply_guardrail_interface = true;
        let base = GuardrailBase::new(settings, params.strict_modes)?;
        let name = base.settings.guardrail_name.as_deref().unwrap_or("None");
        tracing::debug!("Initialized Azure Prompt Shield Guardrail: {name}");
        Ok(Self {
            base,
            transport: AzureGuardrailBase {
                async_handler: params.async_handler,
                api_key: params.api_key,
                api_base: params.api_base,
                api_version: resolved_api_version(params.api_version.as_deref()),
                timeout: params.timeout.unwrap_or(DEFAULT_TIMEOUT),
            },
            cost_tier,
            price_per_1000_text_records,
        })
    }

    // LiteLLM's update_in_memory_litellm_params: everything is resolved and validated before any field moves.
    pub fn update_litellm_params(
        &mut self,
        params: &LitellmParams,
        lookup: &dyn SecretLookup,
    ) -> Result<(), PromptShieldError> {
        let updated = updated_params(params, lookup)?;
        self.base.update_params(params);
        if let Some(api_key) = updated.api_key {
            self.transport.api_key = SecretString::from(api_key);
        }
        if let Some(api_base) = updated.api_base {
            self.transport.api_base = api_base;
        }
        if let Some(api_version) = updated.api_version {
            self.transport.api_version = api_version;
        }
        self.transport.timeout = updated.timeout;
        self.cost_tier = updated.cost_tier;
        self.price_per_1000_text_records = updated.price;
        Ok(())
    }

    pub async fn async_make_request(
        &self,
        user_prompt: &str,
        usage: &mut BillingUsage,
    ) -> Result<(), PromptShieldError> {
        for chunk in split_text_by_words(user_prompt, MAX_TEXT_LENGTH) {
            let body = PromptShieldRequest {
                user_prompt: &chunk,
                documents: &[],
            };
            let response: PromptShieldResponse = self
                .transport
                .post_to_content_safety(ATTACK_PATH, &body)
                .await?;
            usage.record_chunk(&chunk);
            if response.user_prompt_analysis.attack_detected == Some(true) {
                tracing::warn!(
                    "Azure Prompt Shield: Attack detected in chunk of length {}",
                    chunk.chars().count()
                );
                return Err(PromptShieldError::Attack {
                    detection_message: response.user_prompt_analysis.detection_message(),
                });
            }
        }
        Ok(())
    }

    // LiteLLM's apply_guardrail body, with its usage accumulator handed back to the caller.
    pub async fn scan_inputs(
        &self,
        inputs: &GuardrailApiInputs,
        usage: &mut BillingUsage,
    ) -> Result<(), PromptShieldError> {
        *usage = BillingUsage::default();
        for text in inputs.texts.as_deref().unwrap_or_default() {
            if !text.is_empty() {
                self.async_make_request(text, usage).await?;
            }
        }
        Ok(())
    }

    pub async fn async_pre_call_hook(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        call_type: &str,
        usage: &mut BillingUsage,
    ) -> Result<PreCallReturn, PromptShieldError> {
        *usage = BillingUsage::default();
        tracing::debug!(
            "Azure Prompt Shield: Running pre-call prompt scan, on call_type: {call_type}"
        );
        let messages_missing = matches!(data.get("messages"), None | Some(GuardrailValue::Null));
        if !RESPONSES_CALL_TYPES.contains(&call_type) && messages_missing {
            tracing::warn!("Azure Prompt Shield: not running guardrail. No messages in data");
            return Ok(PreCallReturn::Data);
        }
        let user_prompt = get_user_prompt_from_request(data, call_type)?;
        let Some(user_prompt) = user_prompt.filter(|prompt| !prompt.is_empty()) else {
            tracing::warn!("Azure Prompt Shield: No user prompt found");
            return Ok(PreCallReturn::None);
        };
        tracing::debug!("Azure Prompt Shield: User prompt: {user_prompt}");
        self.async_make_request(&user_prompt, usage).await?;
        Ok(PreCallReturn::None)
    }
}

impl GuardrailContract for AzurePromptShieldGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        async move {
            let mut usage = BillingUsage::default();
            self.scan_inputs(&inputs, &mut usage)
                .await
                .map_err(GuardrailFailure::from)?;
            Ok(inputs)
        }
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}

// LiteLLM treats an empty api_version as unset, the same as a missing one.
fn resolved_api_version(configured: Option<&str>) -> String {
    let configured = configured.filter(|version| !version.is_empty());
    resolve_api_version(configured).to_owned()
}

struct UpdatedParams {
    api_key: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
    timeout: Duration,
    cost_tier: Option<CostTier>,
    price: Option<Decimal>,
}

fn updated_params(
    params: &LitellmParams,
    lookup: &dyn SecretLookup,
) -> Result<UpdatedParams, PromptShieldError> {
    let extra = &params.base.extra.0;
    let cost_tier = resolve_cost_tier(extra.get(COST_TIER_KEY), lookup)?;
    let price = resolve_price(extra.get(PRICE_KEY), cost_tier, lookup)?;
    let api_key = params
        .base
        .fields
        .api_key
        .as_ref()
        .map(|key| resolved_reference(key.expose_secret(), lookup))
        .transpose()?;
    let api_base = params
        .base
        .fields
        .api_base
        .as_deref()
        .map(|base| resolved_reference(base, lookup))
        .transpose()?;
    let api_version = updated_api_version(extra.get(API_VERSION_KEY))?;
    let timeout = params
        .base
        .fields
        .timeout
        .map(seconds_timeout)
        .transpose()?
        .unwrap_or(DEFAULT_TIMEOUT);
    Ok(UpdatedParams {
        api_key,
        api_base,
        api_version,
        timeout,
        cost_tier,
        price,
    })
}

fn updated_api_version(
    configured: Option<&GuardrailValue>,
) -> Result<Option<String>, PromptShieldError> {
    match configured {
        None => Ok(None),
        Some(GuardrailValue::Null) => Ok(Some(resolved_api_version(None))),
        Some(GuardrailValue::Text(text)) => Ok(Some(resolved_api_version(Some(text)))),
        Some(other) => Err(PromptShieldError::ApiVersionNotAString {
            value: describe_value(other),
        }),
    }
}

fn seconds_timeout(seconds: f64) -> Result<Duration, PromptShieldError> {
    Duration::try_from_secs_f64(seconds)
        .map_err(|_source| PromptShieldError::InvalidTimeout { seconds })
}
