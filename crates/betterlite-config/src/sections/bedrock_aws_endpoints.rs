use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod aws_config;
mod tests;

// this section reads no YAML key; the names come from the process environment and the AWS profile file
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
    pub session_region: Option<String>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let aws_default_region = lookup.lookup("AWS_DEFAULT_REGION");
        let session_region = aws_default_region
            .clone()
            .or_else(|| aws_config::session_region(lookup));
        Ok(Settings {
            aws_region_name: lookup.lookup("AWS_REGION_NAME"),
            aws_region: lookup.lookup("AWS_REGION"),
            aws_default_region,
            session_region,
            aws_bedrock_runtime_endpoint: lookup.lookup("AWS_BEDROCK_RUNTIME_ENDPOINT"),
        })
    }
}
