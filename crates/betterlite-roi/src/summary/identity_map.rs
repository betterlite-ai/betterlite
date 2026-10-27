use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::settings::{SettingsError, normalize_login};
use crate::summary::models::ROISummary;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "IdentityMapBody")]
pub struct ROIIdentityMapUpdate {
    pub github_login: String,
    // None removes the login from the identity map
    pub email: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityMapBody {
    github_login: String,
    #[serde(deserialize_with = "Option::deserialize")]
    email: Option<String>,
}

impl TryFrom<IdentityMapBody> for ROIIdentityMapUpdate {
    type Error = SettingsError;

    fn try_from(body: IdentityMapBody) -> Result<Self, Self::Error> {
        Ok(Self {
            github_login: normalize_login(&body.github_login)?,
            email: body.email,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROIIdentityMapResponse {
    pub report: Option<ROISummary>,
    pub identity_map: IndexMap<String, String>,
}
