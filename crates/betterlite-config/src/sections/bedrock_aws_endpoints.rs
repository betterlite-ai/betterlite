use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// this section reads no YAML key; the four names come from the process environment
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub aws_region_name: Option<String>,
    pub aws_region: Option<String>,
    pub aws_default_region: Option<String>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            aws_region_name: lookup.lookup("AWS_REGION_NAME"),
            aws_region: lookup.lookup("AWS_REGION"),
            aws_default_region: lookup.lookup("AWS_DEFAULT_REGION"),
            aws_bedrock_runtime_endpoint: lookup.lookup("AWS_BEDROCK_RUNTIME_ENDPOINT"),
        })
    }
}
