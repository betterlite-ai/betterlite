use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const VIDEO_DURATION_ENV: &str = "DEFAULT_GOOGLE_VIDEO_DURATION_SECONDS";

const DEFAULT_VIDEO_DURATION: i64 = 8;

// the duration comes from the environment only, so the section reads no YAML key
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub default_video_duration_seconds: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_video_duration_seconds: DEFAULT_VIDEO_DURATION,
        }
    }
}

// LiteLLM reads this constant as int(getenv(name, 8)) once per process; the loader reads it per load through the same lookup.
impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(text) = lookup.lookup(VIDEO_DURATION_ENV) else {
            return Ok(Settings::default());
        };
        let seconds = betterlite_env::number::<i64>(VIDEO_DURATION_ENV, &text)?
            .unwrap_or(DEFAULT_VIDEO_DURATION);
        Ok(Settings {
            default_video_duration_seconds: seconds,
        })
    }
}
