use betterlite_models::identity::provider_id::ProviderId;

use crate::api::video_selection::error::VideoSelectionError;
use crate::api::video_status_transport::clients::ClientFactories;
use crate::api::video_status_transport::config::StatusConfig;

// B3332's get_provider_video_config(model=None, provider), answering with the provider's status config
pub trait VideoStatusProviders: Send + Sync {
    type Factories: ClientFactories;

    fn video_config(
        &self,
        provider: ProviderId,
    ) -> Result<Option<&StatusConfig<'_, Self::Factories>>, VideoSelectionError>;
}
