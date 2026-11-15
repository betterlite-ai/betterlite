use std::collections::BTreeMap;
use std::error::Error as StdError;

use betterlite_models::decisions::Json;

use crate::message_replay::advisor::strip_advisor_blocks;
use crate::tool_names::wire;
use crate::tool_names::{rewrite_tool_names_in_messages, sanitize_tool_names_in_request};
use crate::tools::wire::ADVISOR_TYPE;

use super::super::beta_headers::{add_optional_betas, tool_kind};
use super::super::code_execution::add_code_execution_tool;
use super::super::convert;
use super::super::error::RequestError;
use super::super::tool_list::tool_entries;
use super::Preparation;

impl<FileFailure> Preparation<'_, FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    // B573 rewrites the declared tools and the OpenAI history in one place, storing the inverse map for responses
    pub(super) fn rewrite_tool_names(&mut self) -> Result<(), RequestError<FileFailure>> {
        let mut record = wire::RequestParams::default();
        if let Some(tools) = self.optional_params.get("tools") {
            record.head.tools = Some(convert::encode(tools)?);
        }
        if let Some(choice) = self.optional_params.get("tool_choice") {
            record.head.tool_choice = Some(convert::encode(choice)?);
        }
        let maps = sanitize_tool_names_in_request(&mut record);
        if maps.forward.is_empty() {
            return Ok(());
        }
        self.write_tool_names(&record)?;
        self.rewrite_history(&maps.forward)?;
        self.tool_name_reverse = maps.reverse;
        Ok(())
    }

    fn write_tool_names(
        &mut self,
        record: &wire::RequestParams,
    ) -> Result<(), RequestError<FileFailure>> {
        if let Some(tools) = &record.head.tools {
            self.optional_params
                .insert("tools".to_owned(), convert::encode(tools)?);
        }
        if let Some(choice) = &record.head.tool_choice {
            self.optional_params
                .insert("tool_choice".to_owned(), convert::encode(choice)?);
        }
        Ok(())
    }

    fn rewrite_history(
        &mut self,
        forward: &BTreeMap<String, String>,
    ) -> Result<(), RequestError<FileFailure>> {
        let records: Vec<wire::Message> = self
            .messages
            .iter()
            .map(convert::encode::<_, wire::Message>)
            .collect::<Result<_, _>>()?;
        let rewritten = rewrite_tool_names_in_messages(&records, forward);
        self.messages = rewritten
            .iter()
            .map(convert::encode)
            .collect::<Result<_, _>>()?;
        Ok(())
    }

    // The header betas, advisor cleanup and code-execution insertion read the same translated message list
    pub(super) fn apply_tool_steps(&mut self) -> Result<(), RequestError<FileFailure>> {
        let mut tools = tool_entries(&self.optional_params)?;
        add_optional_betas(
            &mut self.headers,
            &self.optional_params,
            &tools,
            &self.anthropic_messages,
            &self.provider,
        )?;
        let has_advisor = tools
            .iter()
            .any(|tool| tool_kind(tool) == Some(ADVISOR_TYPE));
        if !has_advisor {
            strip_advisor_blocks(&mut self.anthropic_messages, false);
        }
        add_code_execution_tool(&self.anthropic_messages, &mut tools)?;
        if tools.len() > 1 {
            let encoded: Vec<Json> = tools.iter().map(Json::from).collect();
            self.optional_params
                .insert("tools".to_owned(), Json::List(encoded));
        }
        Ok(())
    }
}
