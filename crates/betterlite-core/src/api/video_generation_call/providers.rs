use crate::api::video_records::{
    // ast-grep-ignore: no-long-names
    VideoCreateOptionalRequestParams,
};
use crate::api::video_selection::VideoConfigTag;
use crate::api::videos_http_transport::client::ClientSource;
use crate::api::videos_http_transport::video_config::{Params, VideoHttpConfig};

// B3332's get_provider_video_config answers a tag, and the caller installs the config and client source it built for it
pub trait VideoGenerationProviders: Send + Sync {
    type Config: VideoGenerationConfig;
    type Source: ClientSource;

    fn video_config(&self, tag: &VideoConfigTag) -> Option<(&Self::Config, &Self::Source)>;
}

// B3331's map_openai_params, which every video provider config owns; drop_params is B198's switch
pub trait VideoGenerationConfig: VideoHttpConfig {
    fn map_openai_params(
        &self,
        // ast-grep-ignore: no-long-names
        options: &VideoCreateOptionalRequestParams,
        model: &str,
        drop_params: bool,
    ) -> Params;
}
