pub mod error;
pub mod headers;
pub mod params;
pub mod reply;
pub mod request;
pub mod stream;

use crate::openai_base_config::Object;

use self::error::NlpCloudConfigError;

mod tests;

// LiteLLM's NLPCloudConfig declares these class values, and a caller may set any of them
const CONFIG_KEYS: [&str; 12] = [
    "max_length",
    "length_no_input",
    "end_sequence",
    "remove_end_sequence",
    "remove_input",
    "bad_words",
    "temperature",
    "top_p",
    "top_k",
    "repetition_penalty",
    "num_beams",
    "num_return_sequences",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NlpCloudConfig {
    defaults: Object,
}

impl NlpCloudConfig {
    pub fn new(defaults: Object) -> Result<Self, NlpCloudConfigError> {
        if let Some(key) = defaults
            .keys()
            .find(|key| !CONFIG_KEYS.contains(&key.as_str()))
        {
            return Err(NlpCloudConfigError::UnknownSetting { key: key.clone() });
        }
        let kept = defaults
            .into_iter()
            .filter(|(_, value)| !value.is_null())
            .collect();
        Ok(Self { defaults: kept })
    }

    // LiteLLM fills a default only when the option key is absent, whatever value the option holds
    pub fn merge_defaults(&self, options: &mut Object) {
        for (key, value) in &self.defaults {
            if options.contains_key(key) {
                continue;
            }
            options.insert(key.clone(), value.clone());
        }
    }
}
