use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const MODE_PATH: &str = "general_settings.ui_access_mode";
const CLAIM_FIELD_PATH: &str = "general_settings.ui_access_mode.sso_group_jwt_field";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    general_settings: RawGeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawGeneralSettings {
    ui_access_mode: Option<RawUiAccessMode>,
}

// a string mode builds no SSO handler, so matching keeps only the shape and discards the text
#[derive(Debug, Deserialize)]
#[serde(from = "String")]
struct ModeText;

impl From<String> for ModeText {
    fn from(_text: String) -> Self {
        Self
    }
}

// LiteLLM builds the SSO handler only for a dict, so a string mode builds none while a list or a number has no declared type
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawUiAccessMode {
    Text(ModeText),
    Mapping(RawGroupProjection),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
struct RawGroupProjection {
    sso_group_jwt_field: Option<RawClaimField>,
    // type and restricted_sso_group belong to B1624, so every other member is accepted and left unread
    #[serde(flatten)]
    _other: BTreeMap<String, Option<IgnoredAny>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawClaimField {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub group_projection: Option<GroupProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupProjection {
    pub sso_group_jwt_field: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(mode) = self.general_settings.ui_access_mode else {
            return Ok(Settings::default());
        };
        match mode {
            RawUiAccessMode::Text(_) => Ok(Settings::default()),
            RawUiAccessMode::Mapping(projection) => Ok(Settings {
                group_projection: Some(GroupProjection {
                    sso_group_jwt_field: claim_field(projection.sso_group_jwt_field, lookup)?,
                }),
            }),
            RawUiAccessMode::Other(_) => Err(ConfigError::InvalidScalar {
                path: MODE_PATH.to_owned(),
                value: false,
            }),
        }
    }
}

// dict.get reads the claim field as None when it is absent or null, and load_config resolves an os.environ reference before that read
fn claim_field<L>(
    configured: Option<RawClaimField>,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match configured {
        None => Ok(None),
        Some(RawClaimField::Text(text)) => {
            Ok(Some(resolve_string(text, CLAIM_FIELD_PATH, lookup)?))
        }
        Some(RawClaimField::Other(_)) => Err(ConfigError::InvalidScalar {
            path: CLAIM_FIELD_PATH.to_owned(),
            value: false,
        }),
    }
}
