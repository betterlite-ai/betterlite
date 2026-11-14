use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::content_filter_records::{BlockCodeAction, ConfidenceThreshold};
use betterlite_config::sections::guardrail_identity::enums::SupportedGuardrailIntegrations;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::lifecycle::{
    BlockCodeExecutionError, BlockCodeExecutionGuardrail, BlockCodeExecutionParams,
    EventHookSelection,
};

pub const INTEGRATION: SupportedGuardrailIntegrations =
    SupportedGuardrailIntegrations::BlockCodeExecution;

pub const INTEGRATION_VALUE: &str = "block_code_execution";

pub const DEFAULT_ACTION: BlockCodeAction = BlockCodeAction::Block;

pub const DEFAULT_CONFIDENCE_THRESHOLD: f64 = 0.5;

pub const DEFAULT_EXECUTION_INTENT: bool = true;

#[derive(Debug, Error)]
pub enum InitializerError {
    #[error("Block Code Execution guardrail requires a guardrail_name")]
    MissingGuardrailName,
    #[error("litellm_params.{field} is not the declared type")]
    InvalidParam { field: String },
    #[error(transparent)]
    Guardrail(#[from] BlockCodeExecutionError),
}

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<BlockCodeExecutionGuardrail>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKey {
    BlockedLanguages,
    Action,
    ConfidenceThreshold,
    DetectExecutionIntent,
    Mode,
    DefaultOn,
}

impl ParamKey {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BlockedLanguages => "blocked_languages",
            Self::Action => "action",
            Self::ConfidenceThreshold => "confidence_threshold",
            Self::DetectExecutionIntent => "detect_execution_intent",
            Self::Mode => "mode",
            Self::DefaultOn => "default_on",
        }
    }
}

#[must_use]
pub fn get_param(params: &LitellmParams, key: ParamKey) -> Option<GuardrailValue> {
    match key {
        ParamKey::Mode => convert(&params.mode),
        ParamKey::DefaultOn => params.base.fields.default_on.map(GuardrailValue::Boolean),
        ParamKey::BlockedLanguages
        | ParamKey::Action
        | ParamKey::ConfidenceThreshold
        | ParamKey::DetectExecutionIntent => None,
    }
    .or_else(|| params.base.extra.0.get(key.name()).cloned())
}

fn convert<T>(value: &T) -> Option<GuardrailValue>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).ok()?;
    let typed = serde_json::from_value(encoded).ok()?;
    Some(typed)
}

fn decode<T>(field: ParamKey, value: &GuardrailValue) -> Result<T, InitializerError>
where
    T: DeserializeOwned,
{
    let invalid = || InitializerError::InvalidParam {
        field: field.name().to_owned(),
    };
    let encoded = serde_json::to_vec(value).map_err(|_not_json| invalid())?;
    serde_json::from_slice(&encoded).map_err(|_wrong_type| invalid())
}

pub fn initialize_guardrail(
    params: &LitellmParams,
    guardrail: &Guardrail,
    strict_modes: bool,
    registry: &mut dyn CallbackRegistry,
) -> Result<Arc<BlockCodeExecutionGuardrail>, InitializerError> {
    if guardrail.guardrail_name.is_empty() {
        return Err(InitializerError::MissingGuardrailName);
    }
    let configured = BlockCodeExecutionGuardrail::new(
        BlockCodeExecutionParams {
            guardrail_name: Some(guardrail.guardrail_name.clone()),
            blocked_languages: blocked_languages(params)?,
            action: action(params)?,
            confidence_threshold: confidence_threshold(params)?,
            detect_execution_intent: read_flag(
                params,
                ParamKey::DetectExecutionIntent,
                DEFAULT_EXECUTION_INTENT,
                None,
            )?,
            event_hook: Some(event_hook(params)?),
            default_on: read_flag(params, ParamKey::DefaultOn, false, Some(false))?,
            clock: SystemTime::now,
        },
        strict_modes,
    )?;
    let instance = Arc::new(configured);
    registry.add_litellm_callback(Arc::clone(&instance));
    Ok(instance)
}

fn blocked_languages(params: &LitellmParams) -> Result<Option<Vec<String>>, InitializerError> {
    match get_param(params, ParamKey::BlockedLanguages) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Array(items)) => {
            let mut languages = Vec::with_capacity(items.len());
            for item in items {
                let GuardrailValue::Text(language) = item else {
                    return Err(InitializerError::InvalidParam {
                        field: ParamKey::BlockedLanguages.name().to_owned(),
                    });
                };
                languages.push(language);
            }
            Ok(Some(languages))
        }
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Object(_),
        ) => Err(InitializerError::InvalidParam {
            field: ParamKey::BlockedLanguages.name().to_owned(),
        }),
    }
}

fn action(params: &LitellmParams) -> Result<BlockCodeAction, InitializerError> {
    let Some(value) = get_param(params, ParamKey::Action) else {
        return Ok(DEFAULT_ACTION);
    };
    decode(ParamKey::Action, &value)
}

fn confidence_threshold(params: &LitellmParams) -> Result<f64, InitializerError> {
    let Some(value) = get_param(params, ParamKey::ConfidenceThreshold) else {
        return Ok(DEFAULT_CONFIDENCE_THRESHOLD);
    };
    let number = threshold_number(&value)?;
    ConfidenceThreshold::try_from(number)
        .map(|threshold| threshold.0)
        .map_err(|_out_of_range| InitializerError::InvalidParam {
            field: ParamKey::ConfidenceThreshold.name().to_owned(),
        })
}

// LiteLLM's config model declares confidence_threshold with ge=0.0 and le=1.0 and a non-optional float, so this check happens where it enters
fn threshold_number(value: &GuardrailValue) -> Result<f64, InitializerError> {
    match value {
        GuardrailValue::Null => Err(InitializerError::InvalidParam {
            field: ParamKey::ConfidenceThreshold.name().to_owned(),
        }),
        GuardrailValue::Text(text) => {
            match betterlite_env::number::<f64>("confidence_threshold", text) {
                Ok(Some(number)) => Ok(number),
                Ok(None) | Err(_) => Err(InitializerError::InvalidParam {
                    field: ParamKey::ConfidenceThreshold.name().to_owned(),
                }),
            }
        }
        GuardrailValue::SignedInteger(_) | GuardrailValue::UnsignedInteger(_) => {
            decode(ParamKey::ConfidenceThreshold, value)
        }
        GuardrailValue::Decimal(number) => Ok(*number),
        GuardrailValue::Boolean(_) | GuardrailValue::Array(_) | GuardrailValue::Object(_) => {
            Err(InitializerError::InvalidParam {
                field: ParamKey::ConfidenceThreshold.name().to_owned(),
            })
        }
    }
}

// an absent key takes its default; a null is invalid where LiteLLM's field is not optional, and default_on allows one
fn read_flag(
    params: &LitellmParams,
    key: ParamKey,
    absent_default: bool,
    null_default: Option<bool>,
) -> Result<bool, InitializerError> {
    match get_param(params, key) {
        None => Ok(absent_default),
        Some(GuardrailValue::Null) => null_default.ok_or_else(|| InitializerError::InvalidParam {
            field: key.name().to_owned(),
        }),
        Some(GuardrailValue::Boolean(flag)) => Ok(flag),
        Some(
            GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        ) => Err(InitializerError::InvalidParam {
            field: key.name().to_owned(),
        }),
    }
}

fn event_hook(params: &LitellmParams) -> Result<EventHookSelection, InitializerError> {
    match &params.mode {
        LitellmParamsMode::One(name) => Ok(EventHookSelection::One(name.clone())),
        LitellmParamsMode::Many(names) => Ok(EventHookSelection::Many(names.clone())),
        LitellmParamsMode::Tagged(_mode) => Err(InitializerError::InvalidParam {
            field: ParamKey::Mode.name().to_owned(),
        }),
    }
}

pub type Initializer = fn(
    &LitellmParams,
    &Guardrail,
    bool,
    &mut dyn CallbackRegistry,
) -> Result<Arc<BlockCodeExecutionGuardrail>, InitializerError>;

pub type GuardrailClass = fn(
    BlockCodeExecutionParams,
    bool,
) -> Result<BlockCodeExecutionGuardrail, BlockCodeExecutionError>;

pub const GUARDRAIL_INITIALIZER_REGISTRY: [(&str, Initializer); 1] =
    [(INTEGRATION_VALUE, initialize_guardrail)];

pub const GUARDRAIL_CLASS_REGISTRY: [(&str, GuardrailClass); 1] =
    [(INTEGRATION_VALUE, BlockCodeExecutionGuardrail::new)];
