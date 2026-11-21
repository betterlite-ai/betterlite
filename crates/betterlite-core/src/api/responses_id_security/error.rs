use std::fmt;

use betterlite_secrets::error::SecretError;
use thiserror::Error;

const UNMANAGED_DETAIL: &str = "Forbidden. This response id was not issued by this proxy, so the proxy cannot tell who owns it. To let keys address responses this proxy did not issue, set general_settings::allow_unmanaged_response_ids to True in the config.yaml file.";
const USER_DETAIL: &str = "Forbidden. The response id is not associated with the user, who this key belongs to. To disable this security feature, set general_settings::disable_responses_id_security to True in the config.yaml file.";
const TEAM_DETAIL: &str = "Forbidden. The response id is not associated with the team, who this key belongs to. To disable this security feature, set general_settings::disable_responses_id_security to True in the config.yaml file.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forbidden {
    Unmanaged,
    User,
    Team,
}

impl Forbidden {
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::Unmanaged => UNMANAGED_DETAIL,
            Self::User => USER_DETAIL,
            Self::Team => TEAM_DETAIL,
        }
    }
}

impl fmt::Display for Forbidden {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Forbidden(Forbidden),
    #[error("the response id is not valid UTF-8")]
    NotUtf8,
    #[error("the managed response id payload has {fields} fields; three are required")]
    MalformedOwnedPayload { fields: usize },
    #[error("the cipher returned a non-text response id")]
    NonTextCiphertext,
    #[error("could not encrypt the response id")]
    Encrypt {
        #[source]
        source: SecretError,
    },
}

impl Error {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Forbidden(_) => 403,
            Self::NotUtf8
            | Self::MalformedOwnedPayload { .. }
            | Self::NonTextCiphertext
            | Self::Encrypt { .. } => 500,
        }
    }
}
