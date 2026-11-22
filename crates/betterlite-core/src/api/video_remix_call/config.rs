use std::str::FromStr as _;
use std::time::Duration;

use betterlite_models::identity::provider_id::ProviderId;

use super::error::VideoCallError;
use crate::api::video_remix_list_transport::config::VideoRemixListConfig;
use crate::api::video_selection::{VideoConfigTag, get_provider_video_config};

// LiteLLM's video signatures default the provider to openai and the timeout to 600 seconds
pub const DEFAULT_PROVIDER: &str = "openai";
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

// The two operations share one module tree; only their call type and their no-config wording differ
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum VideoOperation {
    Remix,
    List,
}

impl VideoOperation {
    pub(crate) const fn call_type(self) -> &'static str {
        match self {
            Self::Remix => "video_remix",
            Self::List => "video_list",
        }
    }
}

// Which B3331 config a provider tag resolves to: B3332 names the class, and the caller installs the adapters it has
pub trait VideoConfigLookup {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn VideoRemixListConfig>;
}

// LlmProviders(custom_llm_provider) parses, get_provider_video_config names the class, then the lookup answers its adapter
pub(crate) fn provider_config<'deps>(
    configs: &'deps dyn VideoConfigLookup,
    provider: &str,
    operation: VideoOperation,
) -> Result<&'deps dyn VideoRemixListConfig, VideoCallError> {
    let parsed =
        ProviderId::from_str(provider).map_err(|_unknown| VideoCallError::UnknownProvider {
            provider: provider.to_owned(),
        })?;
    let tag = match get_provider_video_config(None, parsed) {
        Ok(Some(tag)) => tag,
        Ok(None) => {
            return Err(VideoCallError::Unsupported {
                operation,
                provider: provider.to_owned(),
            });
        }
        // B3332 keeps its own message for a provider whose config is not built yet, such as edenai
        Err(error) => return Err(VideoCallError::Selection(error)),
    };
    configs
        .video_config(&tag)
        .ok_or_else(|| VideoCallError::ConfigMissing {
            provider: provider.to_owned(),
        })
}

// `timeout or DEFAULT_REQUEST_TIMEOUT`: an explicit zero or a none takes B198's request timeout
pub(crate) fn resolved_timeout(
    timeout: Option<f64>,
    request_timeout_seconds: f64,
) -> Result<Duration, VideoCallError> {
    let seconds = timeout
        .filter(|given| *given != 0.0_f64)
        .unwrap_or(request_timeout_seconds);
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| VideoCallError::Timeout { seconds, source })
}
