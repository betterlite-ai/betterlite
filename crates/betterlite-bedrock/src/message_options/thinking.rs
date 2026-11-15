use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;
use num_traits::cast::cast;

use crate::model_routes::{ParamMap, ParamValue};

use super::{
    ADAPTIVE_THINKING, BEDROCK_PROVIDER, CLEAR_THINKING_EDIT, CLEAR_THINKING_LOG_PREFIX,
    MessageOptionsError, OptionsSources,
};

const ADAPTIVE: &str = "adaptive";
const ENABLED: &str = "enabled";
const NON_ADAPTIVE_PATTERNS: [&str; 12] = [
    "opus-4.5", "opus_4.5", "opus-4-5", "opus_4_5", "opus-4.1", "opus_4.1", "opus-4-1", "opus_4_1",
    "opus-4", "opus_4", "sonnet-4", "sonnet_4",
];

#[must_use]
pub async fn supports_extended_thinking<Source, Transport>(
    model: &str,
    sources: &mut OptionsSources<'_, '_, Source, Transport>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if supports_model_capability(
        model,
        ADAPTIVE_THINKING,
        BEDROCK_PROVIDER,
        sources.environment,
        sources.catalogs,
    )
    .await
    {
        return true;
    }
    let model_lower = model.to_lowercase();
    NON_ADAPTIVE_PATTERNS
        .iter()
        .any(|pattern| model_lower.contains(pattern))
}

pub async fn ensure_clear_thinking<Source, Transport>(
    request: &mut ParamMap,
    model: &str,
    sources: &mut OptionsSources<'_, '_, Source, Transport>,
) -> Result<bool, MessageOptionsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if !clears_thinking(request) || !supports_extended_thinking(model, sources).await {
        return Ok(false);
    }
    let adaptive = supports_model_capability(
        model,
        ADAPTIVE_THINKING,
        BEDROCK_PROVIDER,
        sources.environment,
        sources.catalogs,
    )
    .await;
    let thinking = request.get("thinking").cloned();
    if let Some(ParamValue::Mapping(fields)) = &thinking {
        match fields.get("type") {
            Some(ParamValue::Text(kind)) if kind == ADAPTIVE => return Ok(false),
            Some(ParamValue::Text(kind)) if kind == ENABLED && !adaptive => return Ok(false),
            Some(ParamValue::Text(kind)) if kind == ENABLED => {
                let budget_tokens = resolve_clear_thinking_budget(
                    fields.get("budget_tokens"),
                    sources.budgets.bedrock_min,
                )?;
                inject_adaptive_thinking(request, budget_tokens, model, sources.budgets);
                return Ok(true);
            }
            Some(_) | None => log_replacing_thinking(fields),
        }
    }
    Ok(inject_fallback_thinking(
        request,
        adaptive,
        model,
        sources.budgets,
    ))
}

fn clears_thinking(request: &ParamMap) -> bool {
    let Some(ParamValue::Mapping(context_management)) = request.get("context_management") else {
        return false;
    };
    let Some(ParamValue::List(edits)) = context_management.get("edits") else {
        return false;
    };
    edits.iter().any(|edit| {
        matches!(
            edit,
            ParamValue::Mapping(fields)
                if matches!(
                    fields.get("type"),
                    Some(ParamValue::Text(kind)) if kind == CLEAR_THINKING_EDIT
                )
        )
    })
}

fn log_replacing_thinking(fields: &IndexMap<String, ParamValue>) {
    let rendered = serde_json::to_string(fields).unwrap_or_default();
    tracing::debug!(
        "{CLEAR_THINKING_LOG_PREFIX} replacing thinking={rendered} with minimal thinking config"
    );
}

fn inject_fallback_thinking(
    request: &mut ParamMap,
    adaptive: bool,
    model: &str,
    budgets: &Settings,
) -> bool {
    let budget = budgets.bedrock_min;
    if let Some(tokens) = request.get("max_tokens").and_then(python_int)
        && tokens <= budget
    {
        let logged = request
            .get("max_tokens")
            .map_or_else(String::new, log_value);
        tracing::warn!(
            "{CLEAR_THINKING_LOG_PREFIX} max_tokens={logged} is not greater than minimum thinking budget ({budget}); cannot inject thinking safely"
        );
        return false;
    }
    if adaptive {
        inject_adaptive_thinking(request, budget, model, budgets);
        return true;
    }
    let mut minimal = IndexMap::new();
    minimal.insert("type".to_owned(), ParamValue::Text(ENABLED.to_owned()));
    minimal.insert(
        "budget_tokens".to_owned(),
        ParamValue::Number(serde_json::Number::from(budget)),
    );
    request.insert("thinking".to_owned(), ParamValue::Mapping(minimal));
    tracing::debug!("{CLEAR_THINKING_LOG_PREFIX} injected thinking with budget_tokens={budget}");
    true
}

// Python's int() over the caller's budget_tokens: None takes the configured minimum.
pub fn resolve_clear_thinking_budget(
    requested: Option<&ParamValue>,
    minimum: i64,
) -> Result<i64, MessageOptionsError> {
    match requested {
        None | Some(ParamValue::Null) => Ok(minimum),
        Some(ParamValue::Bool(flag)) => Ok(i64::from(*flag)),
        Some(ParamValue::Number(number)) => number
            .as_i64()
            .or_else(|| cast::<f64, i64>(number.as_f64()?))
            .ok_or(MessageOptionsError::BudgetTokens),
        Some(ParamValue::Text(text)) => text
            .trim()
            .parse::<i64>()
            .map_err(|_error| MessageOptionsError::BudgetTokens),
        Some(ParamValue::List(_) | ParamValue::Mapping(_)) => {
            Err(MessageOptionsError::BudgetTokens)
        }
    }
}

#[must_use]
pub const fn effort_from_thinking_budget(budget_tokens: i64, budgets: &Settings) -> &'static str {
    if budget_tokens >= budgets.xhigh {
        return "xhigh";
    }
    if budget_tokens >= budgets.high {
        return "high";
    }
    if budget_tokens >= budgets.medium {
        return "medium";
    }
    "low"
}

pub fn inject_adaptive_thinking(
    request: &mut ParamMap,
    budget_tokens: i64,
    model: &str,
    budgets: &Settings,
) {
    let mut output_config = request
        .get("output_config")
        .and_then(mapping)
        .cloned()
        .unwrap_or_default();
    let derived = effort_from_thinking_budget(budget_tokens, budgets);
    output_config
        .entry("effort".to_owned())
        .or_insert_with(|| ParamValue::Text(derived.to_owned()));
    let logged = output_config
        .get("effort")
        .map_or_else(String::new, log_value);
    let display = request
        .get("thinking")
        .and_then(mapping)
        .and_then(|fields| fields.get("display"))
        .filter(|display| !matches!(display, ParamValue::Null))
        .cloned();
    let mut adaptive = IndexMap::new();
    adaptive.insert("type".to_owned(), ParamValue::Text(ADAPTIVE.to_owned()));
    if let Some(display) = display {
        adaptive.insert("display".to_owned(), display);
    }
    request.insert(
        "output_config".to_owned(),
        ParamValue::Mapping(output_config),
    );
    request.insert("thinking".to_owned(), ParamValue::Mapping(adaptive));
    tracing::debug!(
        "{CLEAR_THINKING_LOG_PREFIX} injected adaptive thinking with effort={logged} for model={model}"
    );
}

const fn mapping(value: &ParamValue) -> Option<&IndexMap<String, ParamValue>> {
    match value {
        ParamValue::Mapping(fields) => Some(fields),
        ParamValue::Null
        | ParamValue::Bool(_)
        | ParamValue::Number(_)
        | ParamValue::Text(_)
        | ParamValue::List(_) => None,
    }
}

// Python's %s renders a scalar as itself; numbers and containers keep their JSON text.
fn log_value(value: &ParamValue) -> String {
    match value {
        ParamValue::Null => "None".to_owned(),
        ParamValue::Bool(flag) => if *flag { "True" } else { "False" }.to_owned(),
        ParamValue::Text(text) => text.clone(),
        ParamValue::Number(_) | ParamValue::List(_) | ParamValue::Mapping(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

// Python's isinstance(max_tokens, int): only a JSON integer, bools included, is a Python int.
fn python_int(value: &ParamValue) -> Option<i64> {
    match value {
        ParamValue::Bool(flag) => Some(i64::from(*flag)),
        ParamValue::Number(number) => number.as_i64(),
        ParamValue::Null | ParamValue::Text(_) | ParamValue::List(_) | ParamValue::Mapping(_) => {
            None
        }
    }
}
