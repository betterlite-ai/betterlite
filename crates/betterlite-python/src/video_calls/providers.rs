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

// what a content and a status call select between, named so each trait method stays one line
type ContentFound<'config> = Selection<'config, OpenAiProviders>;
type StatusFound<'config> = Option<&'config StatusConfig<'config, StatusFactories>>;

// the tag a provider id names, when it is the one tag this family builds a config for
fn openai_tag(provider: ProviderId) -> Result<Option<VideoConfigTag>, VideoSelectionError> {
    Ok(get_provider_video_config(None, provider)?
        .filter(|tag| matches!(tag, VideoConfigTag::OpenAi)))
}

impl VideoGenerationProviders for OpenAiProviders {
    type Config = OpenAiVideoConfig;
    type Source = NativeSource;

    fn video_config(&self, tag: &VideoConfigTag) -> Option<(&Self::Config, &Self::Source)> {
        if matches!(tag, VideoConfigTag::OpenAi) {
            return Some((&self.config, &self.source));
        }
        None
    }
}

impl VideoContentProviders for OpenAiProviders {
    type Config = OpenAiVideoConfig;
    type Source = NativeSource;

    fn video_config(&self, provider: ProviderId) -> Result<ContentFound<'_>, VideoSelectionError> {
        Ok(openai_tag(provider)?.map(|_tag| (&self.config, &self.source)))
    }
}

impl VideoStatusProviders for OpenAiProviders {
    type Factories = StatusFactories;

    fn video_config(&self, provider: ProviderId) -> Result<StatusFound<'_>, VideoSelectionError> {
        Ok(
            openai_tag(provider)?
                .map(|_tag| -> &StatusConfig<'_, StatusFactories> { &self.config }),
        )
    }
}

impl VideoConfigLookup for OpenAiProviders {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn VideoRemixListConfig> {
        if matches!(tag, VideoConfigTag::OpenAi) {
            return Some(&self.config);
        }
        None
    }
}
