use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub jina_ai_api_base: Option<String>,
    pub jina_ai_api_key: Option<SecretString>,
    pub infinity_api_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let set = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let jina_ai_api_key = set("JINA_AI_API_KEY")
            .or_else(|| set("JINA_API_KEY"))
            .or_else(|| set("JINA_AI_TOKEN"));
        Ok(Settings {
            jina_ai_api_base: set("JINA_AI_API_BASE"),
            jina_ai_api_key: jina_ai_api_key.map(SecretString::from),
            infinity_api_key: set("INFINITY_API_KEY").map(SecretString::from),
        })
    }
}
