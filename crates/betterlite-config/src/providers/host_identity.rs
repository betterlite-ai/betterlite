use secrecy::{ExposeSecret as _, SecretString};

use crate::sections::host_identity_settings::Settings;

const CLARIFAI_API_BASE: &str = "https://api.clarifai.com/v2/ext/openai/v1";
const COMPACTIFAI_API_BASE: &str = "https://api.compactif.ai/v1";
const DATAROBOT_API_BASE: &str = "https://app.datarobot.com";
const DATAROBOT_TESTING_KEY: &str = "fake-api-key";

impl Settings {
    #[must_use]
    pub fn resolve_clarifai(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (String, SecretString) {
        let base = resolved_base(api_base, None, CLARIFAI_API_BASE);
        let key = resolved_key(api_key, self.clarifai_api_key.as_ref(), "");
        (base, key)
    }

    #[must_use]
    pub fn resolve_compactifai(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (String, SecretString) {
        let base = resolved_base(api_base, None, COMPACTIFAI_API_BASE);
        let key = resolved_key(api_key, self.compactifai_api_key.as_ref(), "");
        (base, key)
    }

    #[must_use]
    pub fn resolve_datarobot(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (String, SecretString) {
        let base = resolved_base(
            api_base,
            self.datarobot_endpoint.as_deref(),
            DATAROBOT_API_BASE,
        );
        let key = resolved_key(
            api_key,
            self.datarobot_api_token.as_ref(),
            DATAROBOT_TESTING_KEY,
        );
        (base, key)
    }
}

// the pinned providers read the caller's base and key with `or`, so an empty one falls through to the environment
fn resolved_base(explicit: Option<&str>, fallback: Option<&str>, default: &str) -> String {
    explicit
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| fallback.map(str::to_owned))
        .unwrap_or_else(|| default.to_owned())
}

fn resolved_key(
    explicit: Option<&SecretString>,
    fallback: Option<&SecretString>,
    default: &str,
) -> SecretString {
    explicit
        .filter(|key| !key.expose_secret().is_empty())
        .or(fallback)
        .map_or_else(
            || SecretString::from(default.to_owned()),
            SecretString::clone,
        )
}
