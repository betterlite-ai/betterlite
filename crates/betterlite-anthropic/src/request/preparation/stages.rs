use std::collections::BTreeMap;
use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::anthropic_capability_catalog::sampling::{
    SamplingParam, SamplingRequest, apply_sampling_param,
};
use crate::anthropic_capability_catalog::thinking::maybe_drop_disabled_thinking;
use crate::anthropic_capability_catalog::{CatalogSources, OptionalParams};
use crate::capabilities::speed::{SpeedDecision, SpeedParam, SpeedRequest, maybe_drop_speed_param};
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

// The option gates, defaults, tool steps and cleanup run at their transform_request positions
impl<FileFailure> Preparation<'_, FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    pub(super) fn drop_unsupported_speed(
        &mut self,
        catalogs: &CatalogSources<'_>,
    ) -> Result<(), RequestError<FileFailure>> {
        let speed = self
            .optional_params
            .get("speed")
            .map_or(SpeedParam::Absent, SpeedParam::Present);
        let request = SpeedRequest {
            model: self.model,
            speed,
            drop: self.drop,
            custom_llm_provider: self.custom_llm_provider,
        };
        if maybe_drop_speed_param(&request, catalogs)? == SpeedDecision::Drop {
            self.optional_params.shift_remove("speed");
        }
        Ok(())
    }

    pub(super) async fn drop_disabled_thinking<Source, Transport>(
        &mut self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
    ) where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        let thinking = self
            .optional_params
            .get("thinking")
            .filter(|value| !matches!(value, Json::Null))
            .cloned();
        let mut params = OptionalParams {
            reasoning_effort: None,
            thinking,
            output_config: None,
        };
        let provider = self.provider.as_str();
        if maybe_drop_disabled_thinking(self.model, &mut params, provider, environment, catalogs)
            .await
        {
            self.optional_params.shift_remove("thinking");
        }
    }

    pub(super) fn load_config_defaults(&mut self) -> Result<(), RequestError<FileFailure>> {
        let config = self.defaults.get_config(
            Some(self.model),
            self.catalog,
            self.default_chat_max_tokens,
        )?;
        let Json::Object(fields) = config else {
            return Ok(());
        };
        for (key, value) in fields {
            self.optional_params.entry(key).or_insert(value);
        }
        Ok(())
    }

    pub(super) fn drop_internal_options(&mut self) {
        self.optional_params.shift_remove("is_vertex_request");
        self.optional_params.shift_remove("client_metadata");
    }

    pub(super) fn apply_top_k(
        &mut self,
        catalogs: &CatalogSources<'_>,
    ) -> Result<(), RequestError<FileFailure>> {
        let Some(top_k) = self.optional_params.shift_remove("top_k") else {
            return Ok(());
        };
        if matches!(top_k, Json::Null) {
            return Ok(());
        }
        let request = SamplingRequest {
            model: self.model,
            param: SamplingParam::TopK,
            value: &top_k,
            drop: self.drop,
        };
        if let Some(kept) = apply_sampling_param(&request, catalogs)? {
            self.optional_params.insert("top_k".to_owned(), kept);
        }
        Ok(())
    }

    // B573 rewrites the declared tools and the OpenAI history in one place, storing the inverse map for responses
    pub(super) fn rewrite_tool_names(&mut self) -> Result<(), RequestError<FileFailure>> {
        let mut record = wire::RequestParams::default();
        if let Some(tools) = self.optional_params.get("tools") {
            record.head.tools = Some(serde_json::from_str(&serde_json::to_string(tools)?)?);
        }
        if let Some(choice) = self.optional_params.get("tool_choice") {
            record.head.tool_choice = Some(serde_json::from_str(&serde_json::to_string(choice)?)?);
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
            let encoded = serde_json::to_string(tools)?;
            self.optional_params
                .insert("tools".to_owned(), serde_json::from_str(&encoded)?);
        }
        if let Some(choice) = &record.head.tool_choice {
            let encoded = serde_json::to_string(choice)?;
            self.optional_params
                .insert("tool_choice".to_owned(), serde_json::from_str(&encoded)?);
        }
        Ok(())
    }

    fn rewrite_history(
        &mut self,
        forward: &BTreeMap<String, String>,
    ) -> Result<(), RequestError<FileFailure>> {
        let records: Vec<wire::Message> = convert::convert_each(&self.messages)?;
        let rewritten = rewrite_tool_names_in_messages(&records, forward);
        self.messages = convert::convert_each(&rewritten)?;
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
