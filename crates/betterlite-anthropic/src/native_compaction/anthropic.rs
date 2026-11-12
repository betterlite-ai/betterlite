use std::error::Error;
use std::future::Future;

use betterlite_models::decisions::Json;

use super::blocks::native_blocks;
use super::error::NativeCompactionError;
use super::protocol::{
    Compaction, CompactionKind, CompactionParams, CompactionProtocol, ExtraHeaders, Mapping,
    NativeCompactionProvider, RequestKwargs,
};

pub const COMPACT_BETA_HEADER: &str = "compact-2026-09-04";
const CONFLICTS: [&str; 5] = [
    "context_management",
    "response_format",
    "stop",
    "stop_sequences",
    "tool_choice",
];

// B2225's get_model_info answers this; only an explicit true in the model map turns the flag on.
pub trait ModelInfoLookup: Sync {
    type Error: Error + Send + Sync + 'static;

    fn supports_anthropic_compaction(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> impl Future<Output = Result<Option<bool>, Self::Error>> + Send;
}

pub struct AnthropicCompaction<'lookup, Lookup> {
    pub lookup: &'lookup Lookup,
}

impl<Lookup: ModelInfoLookup> NativeCompactionProvider for AnthropicCompaction<'_, Lookup> {
    async fn supports_native_compaction(&self, params: &CompactionParams<'_>) -> bool {
        if !matches!(
            params.custom_llm_provider,
            None | Some("anthropic" | "openai")
        ) {
            return false;
        }
        let without_openai = params.model.strip_prefix("openai/").unwrap_or(params.model);
        let bare_model = without_openai
            .strip_prefix("anthropic/")
            .unwrap_or(without_openai);
        self.lookup
            .supports_anthropic_compaction(bare_model, "anthropic")
            .await
            .is_ok_and(|flag| flag == Some(true))
    }

    fn compatible_defaults(&self, payload: &Mapping) -> bool {
        CONFLICTS
            .iter()
            .all(|key| matches!(payload.get(*key), None | Some(Json::Null)))
    }

    fn request_kwargs(&self) -> RequestKwargs {
        RequestKwargs {
            compaction: Compaction {
                kind: CompactionKind::Summarize,
            },
            extra_headers: ExtraHeaders {
                anthropic_beta: COMPACT_BETA_HEADER,
            },
        }
    }

    // Python's str.strip also drops the four information separators, which char::is_whitespace does not
    fn extract_summary(
        &self,
        protocol: CompactionProtocol,
        response: &Mapping,
    ) -> Result<Option<String>, NativeCompactionError> {
        let blocks = native_blocks(protocol, response)?;
        let [block] = blocks.as_slice() else {
            return Ok(None);
        };
        let (Some(Json::Text(kind)), Some(Json::Text(signature)), Some(Json::Text(content))) = (
            block.get("type"),
            block.get("signature"),
            block.get("content"),
        ) else {
            return Ok(None);
        };
        let blank = content
            .chars()
            .all(|found| found.is_whitespace() || matches!(found, '\u{1c}'..='\u{1f}'));
        Ok((kind == "compaction" && !signature.is_empty() && !blank).then(|| content.clone()))
    }
}
