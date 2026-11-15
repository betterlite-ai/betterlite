use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::marker::PhantomData;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{CapabilityEnvironment, NamedCapability};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_openai::base_helpers::{
    DUMMY_TOOLS, HistoryMessage, assistant_has_thinking, has_tool_calls, last_tool_call_unthought,
};
use betterlite_prompts::anthropic::assembly::{PromptDeps, anthropic_messages_pt};
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::media::inline::{RemoteMediaFetcher, async_inline_remote_media};
use betterlite_prompts::message_content::Message;
use betterlite_prompts::system_placement::{
    place_mid_conversation_system, split_leading_system_run,
};
use indexmap::IndexMap;

use crate::anthropic_capability_catalog::capability::supports_model_capability;
use crate::anthropic_capability_catalog::{CatalogSources, DropParams};
use crate::headers::AnthropicHeaders;
use crate::option_primitives::{OptionDefaults, resolved_provider};
use crate::params::output_config::{self, OutputConfigRequest};
use crate::tools::map_tools;

use super::AnthropicRequest;
use super::PreparedRequest;
use super::convert;
use super::error::RequestError;
use super::media::inlines_remote_media;
use super::metadata::apply_user_id;
use super::system::translate_system_message;

mod option_gates;
mod tools;

const DROP_THINKING_WARNING: &str = "Dropping 'thinking' param because the last assistant message with tool_calls has no thinking_blocks. The model won't use extended thinking for this turn.";

pub(super) struct Preparation<'request, FileFailure> {
    pub(super) model: &'request str,
    pub(super) custom_llm_provider: Option<&'request str>,
    pub(super) provider: String,
    pub(super) messages: Vec<Json>,
    history: Vec<HistoryMessage>,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) headers: AnthropicHeaders,
    pub(super) conversation: Vec<PromptJson>,
    pub(super) anthropic_messages: Vec<PromptJson>,
    pub(super) tool_name_reverse: BTreeMap<String, String>,
    defaults: &'request OptionDefaults,
    catalog: &'request ModelCatalog,
    default_chat_max_tokens: i64,
    control_metadata: Option<Json>,
    strips_billing_metadata: bool,
    modify_params: bool,
    drop: DropParams,
    _file_failure: PhantomData<fn() -> FileFailure>,
}

impl<'request, FileFailure> Preparation<'request, FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    pub(super) fn new(
        request: &AnthropicRequest<'request>,
        messages: Vec<Json>,
    ) -> Result<Self, RequestError<FileFailure>> {
        let history: Vec<HistoryMessage> = convert::convert_each(&messages)?;
        let call_drop = matches!(
            request.litellm_params.get("drop_params"),
            Some(Json::Bool(true))
        );
        Ok(Self {
            model: request.model,
            custom_llm_provider: request.custom_llm_provider,
            provider: resolved_provider(request.custom_llm_provider).to_owned(),
            messages,
            history,
            optional_params: request.optional_params.clone(),
            headers: request.headers.clone(),
            conversation: Vec::new(),
            anthropic_messages: Vec::new(),
            tool_name_reverse: BTreeMap::new(),
            defaults: request.defaults,
            catalog: request.catalog,
            default_chat_max_tokens: request.default_chat_max_tokens,
            control_metadata: request.litellm_params.get("metadata").cloned(),
            strips_billing_metadata: request.strips_billing_metadata,
            modify_params: request.modify_params,
            drop: DropParams {
                global: request.global_drop_params,
                call: call_drop,
            },
            _file_failure: PhantomData,
        })
    }

    pub(super) async fn inline_remote_media<Fetcher>(
        &mut self,
        fetcher: &Fetcher,
    ) -> Result<(), RequestError<FileFailure>>
    where
        Fetcher: RemoteMediaFetcher + Sync,
    {
        let typed: Vec<Message> = convert::convert_each(&self.messages)?;
        let inlined = async_inline_remote_media(&typed, &inlines_remote_media, fetcher).await?;
        if inlined != typed {
            self.messages = convert::convert_each(&inlined)?;
        }
        Ok(())
    }

    pub(super) async fn run<Source, Transport, Fetcher, Blocks, Ids>(
        &mut self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
        prompts: &PromptDeps<'_, Fetcher, Blocks, Ids>,
    ) -> Result<(), RequestError<FileFailure>>
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
        Fetcher: RemoteMediaFetcher + Sync,
        Blocks: FileBlocks<Failure = FileFailure> + Sync,
        Ids: Fn() -> String + Sync,
    {
        self.inject_dummy_tools()?;
        self.drop_unpaired_thinking();
        self.drop_unsupported_speed(catalogs)?;
        self.drop_disabled_thinking(environment, catalogs).await;
        self.rewrite_tool_names()?;
        self.place_system(environment, catalogs).await?;
        self.translate_messages(prompts).await?;
        self.apply_tool_steps()?;
        self.load_config_defaults()?;
        apply_user_id(&mut self.optional_params, self.control_metadata.as_ref())?;
        self.drop_internal_options();
        self.apply_top_k(catalogs)?;
        Ok(())
    }

    fn inject_dummy_tools(&mut self) -> Result<(), RequestError<FileFailure>> {
        if self.optional_params.contains_key("tools") || !has_tool_calls(&self.history) {
            return Ok(());
        }
        let dummy: Vec<PromptJson> = convert::convert_each(&DUMMY_TOOLS)?;
        let mapping = map_tools(&dummy)?;
        self.optional_params
            .insert("tools".to_owned(), convert::encode(&mapping.tools)?);
        Ok(())
    }

    fn drop_unpaired_thinking(&mut self) {
        let thinking = self
            .optional_params
            .get("thinking")
            .is_some_and(|value| !matches!(value, Json::Null));
        if !thinking || !last_tool_call_unthought(&self.history) {
            return;
        }
        if assistant_has_thinking(&self.history) || !self.modify_params {
            return;
        }
        self.optional_params.shift_remove("thinking");
        tracing::warn!("{DROP_THINKING_WARNING}");
    }

    async fn place_system<Source, Transport>(
        &mut self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
    ) -> Result<(), RequestError<FileFailure>>
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        let typed: Vec<Message> = convert::convert_each(&self.messages)?;
        let (leading, later) = split_leading_system_run(&typed);
        let mut leading = leading;
        let blocks = translate_system_message(&mut leading, self.strips_billing_metadata);
        if !blocks.is_empty() {
            self.optional_params
                .insert("system".to_owned(), convert::encode(&blocks)?);
        }
        let key = NamedCapability::MidConversationSystem.key();
        let provider = self.provider.as_str();
        let supported =
            supports_model_capability(self.model, key, provider, environment, catalogs).await;
        let placed = place_mid_conversation_system(&later, supported);
        self.conversation = convert::convert_each(&placed)?;
        Ok(())
    }

    async fn translate_messages<Fetcher, Blocks, Ids>(
        &mut self,
        prompts: &PromptDeps<'_, Fetcher, Blocks, Ids>,
    ) -> Result<(), RequestError<FileFailure>>
    where
        Fetcher: RemoteMediaFetcher + Sync,
        Blocks: FileBlocks<Failure = FileFailure> + Sync,
        Ids: Fn() -> String + Sync,
    {
        let provider = self.provider.as_str();
        self.anthropic_messages = anthropic_messages_pt(
            &self.conversation,
            self.model,
            provider,
            self.modify_params,
            prompts,
        )
        .await
        .map_err(|source| RequestError::Prompt {
            source,
            received: serde_json::to_string(&self.messages).unwrap_or_default(),
        })?;
        Ok(())
    }

    pub(super) async fn finish<Source, Transport>(
        &mut self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
    ) -> Result<PreparedRequest, RequestError<FileFailure>>
    where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        let mut body = IndexMap::new();
        body.insert("model".to_owned(), Json::Text(self.model.to_owned()));
        let messages: Vec<Json> = self
            .anthropic_messages
            .iter()
            .map(convert::params_json)
            .collect();
        body.insert("messages".to_owned(), Json::List(messages));
        for (key, value) in &self.optional_params {
            body.insert(key.clone(), value.clone());
        }
        let request = OutputConfigRequest {
            model: self.model,
            provider: self.provider.as_str(),
            global_drop: self.drop.global,
        };
        output_config::apply(
            &request,
            &mut body,
            &mut self.optional_params,
            environment,
            catalogs,
        )
        .await?;
        Ok(PreparedRequest {
            body,
            headers: self.headers.clone(),
            tool_name_reverse: std::mem::take(&mut self.tool_name_reverse),
        })
    }
}
