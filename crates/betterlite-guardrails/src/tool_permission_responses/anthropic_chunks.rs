use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::guardrail_message_scope::Object;
use crate::tool_permission::types::object_mapping;

const CHAT_COMPLETION_CHUNK: &str = "chat.completion.chunk";
const FUNCTION_KIND: &str = "function";
const DEFAULT_STOP_REASON: &str = "stop";

// The OpenAI chunk an Anthropic event turns into, shaped like LiteLLM's AnthropicModelResponseIterator output.
#[derive(Debug, Serialize)]
pub(super) struct ConvertedChunk<'event> {
    pub(super) id: &'event str,
    pub(super) object: &'static str,
    pub(super) created: i64,
    pub(super) model: &'event str,
    pub(super) choices: Vec<ConvertedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) usage: Option<ConvertedUsage>,
}

#[derive(Debug, Serialize)]
pub(super) struct ConvertedChoice {
    pub(super) index: u32,
    pub(super) delta: ConvertedDelta,
    pub(super) finish_reason: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct ConvertedDelta {
    pub(super) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_calls: Option<Vec<ConvertedToolCall>>,
}

#[derive(Debug, Serialize)]
pub(super) struct ConvertedToolCall {
    pub(super) id: Option<String>,
    pub(super) kind: &'static str,
    pub(super) function: ConvertedFunction,
    pub(super) index: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct ConvertedFunction {
    pub(super) name: Option<String>,
    pub(super) arguments: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ConvertedUsage {
    pub(super) prompt_tokens: i64,
    pub(super) completion_tokens: i64,
    pub(super) total_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_creation_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_read_input_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    None,
    Text,
    ToolUse,
    Other,
}

#[derive(Debug, Default)]
struct UsageState {
    input_tokens: i64,
    output_tokens: i64,
    cache_creation_input_tokens: i64,
    cache_read_input_tokens: i64,
}

impl UsageState {
    // message_delta usage is cumulative but may omit fields message_start reported; a present null resets one to zero.
    fn merge(&mut self, usage: Option<&GuardrailValue>) {
        let Some(usage) = usage.and_then(object_mapping) else {
            return;
        };
        merge_count(usage, "input_tokens", &mut self.input_tokens);
        merge_count(usage, "output_tokens", &mut self.output_tokens);
        merge_count(
            usage,
            "cache_creation_input_tokens",
            &mut self.cache_creation_input_tokens,
        );
        merge_count(
            usage,
            "cache_read_input_tokens",
            &mut self.cache_read_input_tokens,
        );
    }

    fn converted(&self) -> ConvertedUsage {
        let prompt = self
            .input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens);
        ConvertedUsage {
            prompt_tokens: prompt,
            completion_tokens: self.output_tokens,
            total_tokens: prompt.saturating_add(self.output_tokens),
            cache_creation_input_tokens: (self.cache_creation_input_tokens > 0)
                .then_some(self.cache_creation_input_tokens),
            cache_read_input_tokens: (self.cache_read_input_tokens > 0)
                .then_some(self.cache_read_input_tokens),
        }
    }
}

fn merge_count(fields: &Object, key: &str, target: &mut i64) {
    if let Some(value) = fields.get(key) {
        *target = count_value(value);
    }
}

// LiteLLM's calculate_usage keeps int and float counts and reads anything else as zero
pub(super) fn count_value(value: &GuardrailValue) -> i64 {
    match value {
        GuardrailValue::SignedInteger(count) => *count,
        GuardrailValue::UnsignedInteger(count) => i64::try_from(*count).unwrap_or(i64::MAX),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => 0,
    }
}

pub(super) fn text_field<'fields>(fields: &'fields Object, key: &str) -> Option<&'fields str> {
    match fields.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text),
        None | Some(_) => None,
    }
}

fn object_field<'fields>(fields: &'fields Object, key: &str) -> Option<&'fields Object> {
    let value = fields.get(key)?;
    object_mapping(value)
}

// LiteLLM's map_finish_reason for the reasons an Anthropic message_delta carries
fn mapped_finish_reason(reason: &str) -> &'static str {
    match reason {
        "stop_sequence" | "end_turn" => "stop",
        "max_tokens" | "compaction" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

struct Converter<'event> {
    id: &'event str,
    model: &'event str,
    now: i64,
    tool_index: i64,
    block: BlockKind,
    arguments_seen: bool,
    arguments: String,
    usage: UsageState,
    chunks: Vec<ConvertedChunk<'event>>,
}

impl<'event> Converter<'event> {
    fn push(
        &mut self,
        content: String,
        tool_calls: Option<Vec<ConvertedToolCall>>,
        finish_reason: Option<String>,
        usage: Option<ConvertedUsage>,
    ) {
        self.chunks.push(ConvertedChunk {
            id: self.id,
            object: CHAT_COMPLETION_CHUNK,
            created: self.now,
            model: self.model,
            choices: vec![ConvertedChoice {
                index: 0,
                delta: ConvertedDelta {
                    content,
                    tool_calls,
                },
                finish_reason,
            }],
            usage,
        });
    }

    // LiteLLM's chunk_parser for the events the tool permission check reads
    fn event(&mut self, event: &Object) -> Option<()> {
        let kind = text_field(event, "type").unwrap_or("");
        match kind {
            "message_start" => self.message_start(event),
            "content_block_start" => self.content_block_start(event),
            "content_block_delta" => self.content_block_delta(event),
            "content_block_stop" => {
                self.content_block_stop();
                Some(())
            }
            "message_delta" => self.message_delta(event),
            // The iterator raises on an error event, which the assembly reports as no response at all.
            "error" => None,
            _ => {
                self.push(String::new(), None, None, None);
                Some(())
            }
        }
    }

    fn message_start(&mut self, event: &Object) -> Option<()> {
        let message = object_field(event, "message")?;
        self.usage.merge(message.get("usage"));
        let usage = self.usage.converted();
        self.push(String::new(), None, None, Some(usage));
        Some(())
    }

    fn content_block_start(&mut self, event: &Object) -> Option<()> {
        let block = object_field(event, "content_block")?;
        let kind = text_field(block, "type")?;
        self.block = match kind {
            "text" => {
                let text = text_field(block, "text").unwrap_or("");
                self.push(text.to_owned(), None, None, None);
                BlockKind::Text
            }
            "tool_use" | "server_tool_use" => {
                self.tool_index = self.tool_index.saturating_add(1);
                self.arguments_seen = false;
                self.arguments.clear();
                let call = ConvertedToolCall {
                    id: text_field(block, "id").map(str::to_owned),
                    kind: FUNCTION_KIND,
                    function: ConvertedFunction {
                        name: text_field(block, "name").map(str::to_owned),
                        arguments: String::new(),
                    },
                    index: u32::try_from(self.tool_index).ok()?,
                };
                self.push(String::new(), Some(vec![call]), None, None);
                BlockKind::ToolUse
            }
            _ => {
                self.push(String::new(), None, None, None);
                BlockKind::Other
            }
        };
        Some(())
    }

    fn content_block_delta(&mut self, event: &Object) -> Option<()> {
        let delta = object_field(event, "delta")?;
        if let Some(text) = text_field(delta, "text") {
            self.push(text.to_owned(), None, None, None);
            return Some(());
        }
        if let Some(partial) = text_field(delta, "partial_json")
            && self.block == BlockKind::ToolUse
        {
            self.tool_delta(partial)?;
            return Some(());
        }
        self.push(String::new(), None, None, None);
        Some(())
    }

    fn tool_delta(&mut self, partial: &str) -> Option<()> {
        self.arguments_seen = true;
        self.arguments.push_str(partial);
        let call = ConvertedToolCall {
            id: None,
            kind: FUNCTION_KIND,
            function: ConvertedFunction {
                name: None,
                arguments: partial.to_owned(),
            },
            index: u32::try_from(self.tool_index).ok()?,
        };
        self.push(String::new(), Some(vec![call]), None, None);
        Some(())
    }

    // A tool_use block whose streamed arguments were all empty still needs valid JSON, so it closes with "{}".
    fn content_block_stop(&mut self) {
        let mut tool_calls = None;
        if self.block == BlockKind::ToolUse
            && self.arguments_seen
            && self.arguments.is_empty()
            && let Ok(index) = u32::try_from(self.tool_index)
        {
            tool_calls = Some(vec![ConvertedToolCall {
                id: None,
                kind: FUNCTION_KIND,
                function: ConvertedFunction {
                    name: None,
                    arguments: "{}".to_owned(),
                },
                index,
            }]);
        }
        self.block = BlockKind::None;
        self.push(String::new(), tool_calls, None, None);
    }

    fn message_delta(&mut self, event: &Object) -> Option<()> {
        let delta = object_field(event, "delta")?;
        let stop_reason = text_field(delta, "stop_reason")
            .filter(|reason| !reason.is_empty())
            .unwrap_or(DEFAULT_STOP_REASON);
        let finish_reason = mapped_finish_reason(stop_reason).to_owned();
        self.usage.merge(event.get("usage"));
        let usage = self.usage.converted();
        self.push(String::new(), None, Some(finish_reason), Some(usage));
        Some(())
    }
}

// LiteLLM's AnthropicModelResponseIterator over a parsed stream, with no message_start meaning no response to assemble
#[must_use]
pub(super) fn convert_anthropic_events<'event>(
    events: &'event [Object],
    now: i64,
) -> Option<Vec<ConvertedChunk<'event>>> {
    let start = events
        .iter()
        .find(|event| text_field(event, "type") == Some("message_start"))?;
    let message = object_field(start, "message")?;
    let mut converter = Converter {
        id: text_field(message, "id").unwrap_or(""),
        model: text_field(message, "model").unwrap_or(""),
        now,
        tool_index: -1,
        block: BlockKind::None,
        arguments_seen: false,
        arguments: String::new(),
        usage: UsageState::default(),
        chunks: Vec::new(),
    };
    for event in events {
        converter.event(event)?;
    }
    Some(converter.chunks)
}
