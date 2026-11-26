use betterlite_core::api::video_content_call::providers::{Selection, VideoContentProviders};
use betterlite_core::api::video_generation_call::providers::VideoGenerationProviders;
use betterlite_core::api::video_remix_call::config::VideoConfigLookup;
use betterlite_core::api::video_remix_list_transport::config::VideoRemixListConfig;
use betterlite_core::api::video_selection::error::VideoSelectionError;
use betterlite_core::api::video_selection::{VideoConfigTag, get_provider_video_config};
use betterlite_core::api::video_status_call::providers::VideoStatusProviders;
use betterlite_core::api::video_status_transport::config::StatusConfig;
use betterlite_models::identity::provider_id::ProviderId;

use super::adapter::OpenAiVideoConfig;
use super::clients::{NativeSource, StatusFactories};

// The one video config LiteLLM builds for OpenAI, and the pool every operation takes its clients from
pub(crate) struct OpenAiProviders {
    pub(crate) config: OpenAiVideoConfig,
    pub(crate) source: NativeSource,
}

impl OpenAiProviders {
    // only the OpenAI tag has an adapter here; another provider's config belongs to its own task
    fn openai(&self, tag: &VideoConfigTag) -> bool {
        matches!(tag, VideoConfigTag::OpenAi)
    }
}

impl VideoGenerationProviders for OpenAiProviders {
    type Config = OpenAiVideoConfig;
    type Source = NativeSource;

    fn video_config(&self, tag: &VideoConfigTag) -> Option<(&Self::Config, &Self::Source)> {
        if self.openai(tag) {
            return Some((&self.config, &self.source));
        }
        None
    }
}

impl VideoContentProviders for OpenAiProviders {
    type Config = OpenAiVideoConfig;
    type Source = NativeSource;

    fn video_config(
        &self,
        provider: ProviderId,
    ) -> Result<Selection<'_, Self>, VideoSelectionError> {
        let found = get_provider_video_config(None, provider)?;
        Ok(found
            .filter(|tag| self.openai(tag))
            .map(|_tag| (&self.config, &self.source)))
    }
}

impl VideoStatusProviders for OpenAiProviders {
    type Factories = StatusFactories;

    fn video_config(
        &self,
        provider: ProviderId,
    ) -> Result<Option<&StatusConfig<'_, Self::Factories>>, VideoSelectionError> {
        let found = get_provider_video_config(None, provider)?;
        Ok(found
            .filter(|tag| self.openai(tag))
            .map(|_tag| &self.config as &StatusConfig<'_, StatusFactories>))
    }
}

impl VideoConfigLookup for OpenAiProviders {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn VideoRemixListConfig> {
        if self.openai(tag) {
            return Some(&self.config as &dyn VideoRemixListConfig);
        }
        None
    }
}
