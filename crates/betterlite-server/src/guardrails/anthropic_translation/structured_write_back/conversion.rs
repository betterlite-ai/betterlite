use betterlite_prompts::anthropic::assembly::{PromptDeps, anthropic_messages_pt};
use betterlite_prompts::anthropic_history::groups::group_tool_exchanges;
use betterlite_prompts::anthropic_invokes::file_blocks::RemoteFileBlocks;
use betterlite_prompts::arguments::json::Json as PromptJson;
use serde_json::Value;
use uuid::Uuid;

use super::deps::NoMedia;
use super::error::StructuredWriteBackError;
use super::{CACHE_CONTROL, CONTENT, THINKING, TYPE, is_system, system_row_to_anthropic};

// LiteLLM's modify_params is on: an empty list still yields the placeholder turn.
const PROVIDER: &str = "anthropic";

pub(super) fn prompt_rows(rows: &[Value]) -> Result<Vec<PromptJson>, StructuredWriteBackError> {
    rows.iter()
        .map(|row| {
            serde_json::from_value::<PromptJson>(row.clone())
                .map_err(StructuredWriteBackError::Encoding)
        })
        .collect()
}

async fn convert(
    rows: &[PromptJson],
    model: &str,
) -> Result<Vec<PromptJson>, StructuredWriteBackError> {
    let media = NoMedia;
    let blocks = RemoteFileBlocks { fetcher: &media };
    let new_id = || Uuid::new_v4().to_string();
    let deps = PromptDeps {
        fetcher: &media,
        blocks: &blocks,
        new_id: &new_id,
    };
    anthropic_messages_pt(rows, model, PROVIDER, true, &deps)
        .await
        .map_err(StructuredWriteBackError::Conversion)
}

pub(super) async fn convert_grouped(
    run: &[Value],
    model: &str,
) -> Result<Vec<Value>, StructuredWriteBackError> {
    let rows = prompt_rows(run)?;
    let mut converted = Vec::new();
    for group in group_tool_exchanges(&rows) {
        let group_rows = group
            .iter()
            .map(|index| rows.get(*index).cloned())
            .collect::<Option<Vec<PromptJson>>>();
        let Some(group_rows) = group_rows else {
            return Err(StructuredWriteBackError::RowMissing);
        };
        for row in convert(&group_rows, model).await? {
            converted.push(serde_json::to_value(row).map_err(StructuredWriteBackError::Encoding)?);
        }
    }
    Ok(converted)
}

pub(super) async fn placeholder(model: &str) -> Result<Vec<Value>, StructuredWriteBackError> {
    let rows = Vec::new();
    convert(&rows, model)
        .await?
        .into_iter()
        .map(|row| serde_json::to_value(row).map_err(StructuredWriteBackError::Encoding))
        .collect()
}

pub(super) struct Conversion<'input> {
    model: &'input str,
    hoisted: Option<&'input Value>,
    preserve_system_messages: bool,
    hoisted_dropped: bool,
    converted: Vec<Value>,
}

impl<'input> Conversion<'input> {
    pub(super) const fn new(
        model: &'input str,
        hoisted: Option<&'input Value>,
        preserve_system_messages: bool,
        hoisted_dropped: bool,
    ) -> Self {
        Self {
            model,
            hoisted,
            preserve_system_messages,
            hoisted_dropped,
            converted: Vec::new(),
        }
    }

    pub(super) async fn walk(&mut self, rows: &[Value]) -> Result<(), StructuredWriteBackError> {
        let mut run: Vec<Value> = Vec::new();
        for message in rows {
            if !is_system(message) {
                run.push(message.clone());
                continue;
            }
            self.convert_run(&run).await?;
            run.clear();
            if !self.hoisted_dropped && self.hoisted.is_some_and(|hoisted| message == hoisted) {
                self.hoisted_dropped = true;
                continue;
            }
            if self.preserve_system_messages
                && let Some(row) = system_row_to_anthropic(message)
            {
                self.converted.push(Value::Object(row));
            }
        }
        self.convert_run(&run).await
    }

    pub(super) async fn finish(mut self) -> Result<Vec<Value>, StructuredWriteBackError> {
        if !self.converted.iter().any(|message| !is_system(message)) {
            self.converted.extend(placeholder(self.model).await?);
        }
        strip_thinking_cache_control(&mut self.converted);
        Ok(self.converted)
    }

    async fn convert_run(&mut self, run: &[Value]) -> Result<(), StructuredWriteBackError> {
        if run.is_empty() {
            return Ok(());
        }
        let converted = convert_grouped(run, self.model).await?;
        self.converted.extend(converted);
        Ok(())
    }
}

fn strip_thinking_cache_control(converted: &mut [Value]) {
    for message in converted {
        let Some(Value::Array(blocks)) = message.get_mut(CONTENT) else {
            continue;
        };
        for block in blocks {
            let Some(fields) = block.as_object_mut() else {
                continue;
            };
            if fields.get(TYPE).and_then(Value::as_str) == Some(THINKING) {
                fields.remove(CACHE_CONTROL);
            }
        }
    }
}
