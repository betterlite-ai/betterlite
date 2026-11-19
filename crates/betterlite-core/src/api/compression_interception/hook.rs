use std::sync::Arc;

use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;
use uuid::Uuid;

use super::cache::{Clock, CompressionCache};
use super::engine::{CompressionCall, CompressionEngine, CompressionOutput};
use super::savings::{compression_savings_from_counts, record_compression_savings};
use super::tools::{has_retrieval_tool, merge_tools};
use crate::routes::CallType;

const MESSAGES_FIELD: &str = "messages";
const MODEL_FIELD: &str = "model";
const TOOLS_FIELD: &str = "tools";
const CALL_ID_FIELD: &str = "litellm_call_id";
const AGENTIC_LOOP_DEPTH_FIELD: &str = "_agentic_loop_depth";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookResult {
    Applied,
    Skipped,
}

#[derive(Debug, Error)]
pub enum CompressionHookError<Failure> {
    #[error("the compression engine failed")]
    Engine(#[source] Failure),
    #[error("the {field} field must be a list of tools")]
    ToolsNotAList { field: &'static str },
}

pub struct CompressionInterception<Engine> {
    pub settings: Settings,
    pub cache: CompressionCache,
    engine: Arc<Engine>,
}

impl<Engine> CompressionInterception<Engine>
where
    Engine: CompressionEngine,
{
    #[must_use]
    pub fn new(settings: Settings, engine: Arc<Engine>) -> Self {
        Self {
            settings,
            cache: CompressionCache::default(),
            engine,
        }
    }

    #[must_use]
    pub fn with_clock(settings: Settings, engine: Arc<Engine>, clock: Clock) -> Self {
        Self {
            settings,
            cache: CompressionCache::with_clock(clock),
            engine,
        }
    }

    pub async fn pre_call_deployment_hook(
        &self,
        request: &mut IndexMap<String, Json>,
        call_type: Option<CallType>,
    ) -> Result<HookResult, CompressionHookError<Engine::Failure>> {
        if !self.settings.enabled {
            return Ok(HookResult::Skipped);
        }
        if call_type.is_some_and(|call_type| call_type != CallType::AnthropicMessages) {
            return Ok(HookResult::Skipped);
        }
        if agentic_loop_depth(request) > 0 {
            return Ok(HookResult::Skipped);
        }
        let Some(Json::List(messages)) = request.get(MESSAGES_FIELD) else {
            return Ok(HookResult::Skipped);
        };
        let Some(Json::Text(model)) = request.get(MODEL_FIELD) else {
            return Ok(HookResult::Skipped);
        };
        if has_retrieval_tool(request.get(TOOLS_FIELD)) {
            return Ok(HookResult::Skipped);
        }
        self.cache.prune();
        let output = {
            let call = self.compression_call(messages.as_slice(), model);
            self.engine
                .compress(&call)
                .await
                .map_err(CompressionHookError::Engine)?
        };
        self.apply_output(request, output)?;
        Ok(HookResult::Applied)
    }

    #[must_use]
    pub fn get_cache(&self, call_id: Option<&str>) -> IndexMap<String, String> {
        let Some(call_id) = call_id else {
            return IndexMap::new();
        };
        match self.cache.entry(call_id) {
            Some(entry) => entry.content,
            None => IndexMap::new(),
        }
    }

    fn compression_call<'call>(
        &'call self,
        messages: &'call [Json],
        model: &'call str,
    ) -> CompressionCall<'call> {
        CompressionCall {
            model,
            call_type: CallType::AnthropicMessages,
            messages,
            compression_trigger: self.settings.compression_trigger,
            compression_target: self.settings.compression_target,
            embedding_model: self.settings.embedding_model.as_deref(),
            embedding_model_params: self.settings.embedding_model_params.as_ref(),
        }
    }

    fn apply_output(
        &self,
        request: &mut IndexMap<String, Json>,
        output: CompressionOutput,
    ) -> Result<(), CompressionHookError<Engine::Failure>> {
        if output.cache.is_empty() {
            log_skip(&output);
            return Ok(());
        }
        let cached_keys = output.cache.len();
        request.insert(MESSAGES_FIELD.to_owned(), Json::List(output.messages));
        if !output.tools.is_empty() {
            let merged = {
                let existing: &[Json] = match request.get(TOOLS_FIELD) {
                    None | Some(Json::Null) => &[],
                    Some(Json::List(tools)) => tools.as_slice(),
                    Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                        return Err(CompressionHookError::ToolsNotAList { field: TOOLS_FIELD });
                    }
                };
                merge_tools(existing, &output.tools)
            };
            request.insert(TOOLS_FIELD.to_owned(), Json::List(merged));
        }
        let call_id = self.store_content(request, &output.cache);
        if let Some(savings) =
            compression_savings_from_counts(&output.original_tokens, &output.compressed_tokens)
        {
            record_compression_savings(request, &savings);
        }
        tracing::debug!(
            kind = "compressed",
            call_id = call_id.as_str(),
            original = %count_text(&output.original_tokens),
            compressed = %count_text(&output.compressed_tokens),
            cached_keys,
            "CompressionInterception: compressed request"
        );
        Ok(())
    }

    fn store_content(
        &self,
        request: &mut IndexMap<String, Json>,
        content: &IndexMap<String, String>,
    ) -> String {
        let existing = match request.get(CALL_ID_FIELD) {
            Some(Json::Text(call_id)) if !call_id.is_empty() => Some(call_id.clone()),
            None
            | Some(
                Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::List(_)
                | Json::Object(_)
                | Json::Text(_),
            ) => None,
        };
        let call_id = match existing {
            Some(call_id) => call_id,
            None => {
                let generated = Uuid::new_v4().to_string();
                request.insert(CALL_ID_FIELD.to_owned(), Json::Text(generated.clone()));
                generated
            }
        };
        self.cache.store(call_id.clone(), content.clone());
        call_id
    }
}

#[must_use]
pub fn resolve_call_id<'request>(
    logging_call_id: Option<&'request str>,
    request: &'request IndexMap<String, Json>,
) -> Option<&'request str> {
    if let Some(call_id) = logging_call_id.filter(|call_id| !call_id.is_empty()) {
        return Some(call_id);
    }
    match request.get(CALL_ID_FIELD) {
        Some(Json::Text(call_id)) => Some(call_id.as_str()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn agentic_loop_depth(request: &IndexMap<String, Json>) -> i64 {
    match request.get(AGENTIC_LOOP_DEPTH_FIELD) {
        Some(Json::Number(depth)) => depth.as_i64().unwrap_or_default(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => 0,
    }
}

fn log_skip(output: &CompressionOutput) {
    let Some(reason) = output.skipped_reason else {
        return;
    };
    tracing::debug!(
        kind = "skipped",
        reason = reason.as_str(),
        original = %count_text(&output.original_tokens),
        compressed = %count_text(&output.compressed_tokens),
        "CompressionInterception: compression skipped"
    );
}

// The source prints both counts with %d; betterlite prints whatever compress returned instead.
fn count_text(value: &Json) -> String {
    match value {
        Json::Null => "null".to_owned(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) => "list".to_owned(),
        Json::Object(_) => "object".to_owned(),
    }
}
