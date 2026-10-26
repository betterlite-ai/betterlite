use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::settings::normalize_login;
use crate::summary::models::ROISummary;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ROIIdentityMapUpdate {
    #[serde(deserialize_with = "deserialize_login")]
    pub github_login: String,
    // null removes the login from the identity map
    #[serde(deserialize_with = "Option::deserialize")]
    pub email: Option<String>,
}

fn deserialize_login<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let login = String::deserialize(deserializer)?;
    normalize_login(&login).map_err(D::Error::custom)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROIIdentityMapResponse {
    pub report: Option<ROISummary>,
    pub identity_map: IndexMap<String, String>,
}
