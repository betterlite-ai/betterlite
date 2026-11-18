use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::maybe_drop_disabled_thinking;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_openai::base_helpers::{
    HistoryMessage, assistant_has_thinking, last_tool_call_unthought,
};
use betterlite_prompts::message_content::Message;

use crate::converse_params::wire::transcode;
use crate::converse_request::error::ConverseRequestError;
use crate::model_routes::{ParamMap, ParamValue};

const BEDROCK_PROVIDER: &str = "bedrock";
const DROP_THINKING_WARNING: &str = "Dropping 'thinking' param because the last assistant message with tool_calls has no thinking_blocks. The model won't use extended thinking for this turn.";
const THINKING_FIELD: &str = "thinking";

// LiteLLM drops the param on a turn whose own tool history carries no thinking, even without tools= present
pub(super) fn apply_history_policy(
    messages: &[Message],
    optional_params: &mut ParamMap,
    modify_params: bool,
) -> Result<(), ConverseRequestError> {
    let thinking = optional_params.get(THINKING_FIELD);
    if thinking.is_none_or(|value| matches!(value, ParamValue::Null)) {
        return Ok(());
    }
    let history: Vec<HistoryMessage> = transcode(messages)?;
    if !last_tool_call_unthought(&history) || assistant_has_thinking(&history) || !modify_params {
        return Ok(());
    }
    optional_params.shift_remove(THINKING_FIELD);
    tracing::warn!("{DROP_THINKING_WARNING}");
    Ok(())
}

// AnthropicModelInfo.maybe_drop_disabled_thinking with the provider the pinned module passes
pub(super) async fn drop_disabled<Source, Transport>(
    model: &str,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<(), ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let Some(thinking) = optional_params.get(THINKING_FIELD) else {
        return Ok(());
    };
    let encoded: Json = transcode(thinking)?;
    let mut params = OptionalParams {
        reasoning_effort: None,
        thinking: Some(encoded),
        output_config: None,
    };
    let dropped =
        maybe_drop_disabled_thinking(model, &mut params, BEDROCK_PROVIDER, environment, catalogs)
            .await;
    if dropped {
        optional_params.shift_remove(THINKING_FIELD);
    }
    Ok(())
}
