use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::{cometapi_embedding_credentials, openai_like_settings};
use secrecy::SecretString;

pub const DEFAULT_COMETAPI_API_BASE: &str = "https://api.cometapi.com/v1";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// litellm.api_base, litellm.api_key, litellm.openai_like_key and litellm.cometapi_key, supplied by the dispatch
#[derive(Debug, Clone, Default)]
pub struct Globals {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_like_key: Option<SecretString>,
    pub cometapi_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub openai_like: openai_like_settings::Settings,
    pub cometapi_credentials: cometapi_embedding_credentials::Settings,
    pub cometapi_api_base: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingFamily {
    OpenAiLike,
    Cometapi,
}

#[derive(Debug, Clone, Default)]
pub struct Identity {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

impl Settings {
    // a supplied value wins even when empty; only absence falls through the globals to the env-backed sections
    #[must_use]
    pub fn identity(
        &self,
        family: EmbeddingFamily,
        globals: &Globals,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> Identity {
        let supplied_base = api_base
            .map(str::to_owned)
            .or_else(|| globals.api_base.clone());
        let supplied_key = api_key.cloned();
        match family {
            EmbeddingFamily::OpenAiLike => Identity {
                api_base: supplied_base.or_else(|| self.openai_like.api_base.clone()),
                api_key: supplied_key
                    .or_else(|| globals.api_key.clone())
                    .or_else(|| globals.openai_like_key.clone())
                    .or_else(|| self.openai_like.api_key.clone()),
            },
            // CometAPI puts the general global key last, after its own alias and the environment
            EmbeddingFamily::Cometapi => Identity {
                api_base: Some(
                    supplied_base
                        .or_else(|| self.cometapi_api_base.clone())
                        .unwrap_or_else(|| DEFAULT_COMETAPI_API_BASE.to_owned()),
                ),
                api_key: supplied_key
                    .or_else(|| globals.cometapi_key.clone())
                    .or_else(|| self.cometapi_credentials.api_key.clone())
                    .or_else(|| globals.api_key.clone()),
            },
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let openai_like = openai_like_settings::Raw::default().convert(lookup)?;
        let cometapi_credentials =
            cometapi_embedding_credentials::Raw::default().convert(lookup)?;
        Ok(Settings {
            openai_like,
            cometapi_credentials,
            cometapi_api_base: lookup
                .lookup("COMETAPI_API_BASE")
                .filter(|value| !value.is_empty()),
        })
    }
}
