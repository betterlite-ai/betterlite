use std::num::NonZeroUsize;

use betterlite_config::sections::token_counting::Settings as ConfigSettings;

use crate::engine::limits::EncodeLimits;

use super::error::CountError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub disabled: bool,
    pub max_exact_chars: NonZeroUsize,
    pub chunk_chars: NonZeroUsize,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_config(&ConfigSettings::default())
    }
}

impl Settings {
    #[must_use]
    pub const fn from_config(settings: &ConfigSettings) -> Self {
        Self {
            disabled: settings.disable_token_counter,
            max_exact_chars: settings.max_exact_chars,
            chunk_chars: settings.chunk_chars,
        }
    }

    pub fn encode_limits(&self) -> Result<EncodeLimits, CountError> {
        // an exactly counted text is at most the exact threshold in scalar values, and UTF-8 takes at most four bytes each
        let limit = self
            .max_exact_chars
            .get()
            .checked_mul(4)
            .ok_or(CountError::LimitsOverflow)?;
        Ok(EncodeLimits {
            input_bytes: limit,
            output_ids: limit,
        })
    }
}
