use crate::loader::{ConfigError, EnvironmentLookup};

pub const VIDEO_DURATION_ENV: &str = "DEFAULT_GOOGLE_VIDEO_DURATION_SECONDS";

const DEFAULT_VIDEO_DURATION: i64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexSettings {
    pub default_video_duration_seconds: i64,
}

impl Default for VertexSettings {
    fn default() -> Self {
        Self {
            default_video_duration_seconds: DEFAULT_VIDEO_DURATION,
        }
    }
}

// LiteLLM reads this constant as int(getenv(name, 8)) once per process; the loader reads it per call through the same lookup.
pub fn load_vertex_settings<L>(lookup: &L) -> Result<VertexSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(VIDEO_DURATION_ENV) else {
        return Ok(VertexSettings::default());
    };
    let seconds =
        betterlite_env::number::<i64>(VIDEO_DURATION_ENV, &text)?.unwrap_or(DEFAULT_VIDEO_DURATION);
    Ok(VertexSettings {
        default_video_duration_seconds: seconds,
    })
}
