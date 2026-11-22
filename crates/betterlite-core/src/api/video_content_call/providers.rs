use betterlite_models::identity::provider_id::ProviderId;

use crate::api::video_selection::error::VideoSelectionError;
use crate::api::videos_http_transport::client::ClientSource;
use crate::api::videos_http_transport::video_config::VideoHttpConfig;

// the provider's config and client source, when it has video support
pub type Selection<'providers, Providers> = Option<(
    &'providers <Providers as VideoContentProviders>::Config,
    &'providers <Providers as VideoContentProviders>::Source,
)>;

// B3332's get_provider_video_config(model=None, provider), answering with the provider's config and its client source
pub trait VideoContentProviders: Send + Sync {
    type Config: VideoHttpConfig;
    type Source: ClientSource;

    fn video_config(
        &self,
        provider: ProviderId,
    ) -> Result<Selection<'_, Self>, VideoSelectionError>;
}
