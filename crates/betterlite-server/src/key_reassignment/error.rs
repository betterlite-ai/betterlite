use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_db::key_repository::TokenRepositoryError;
use betterlite_db::key_repository::decode::TokenDecodeError;
use betterlite_db::key_repository::table::TokenTableError;
use betterlite_db::resource_repository::budget::BudgetRepositoryError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::key_access::error::DetailShape;
use crate::key_patch_fields::error::KeyPatchError;
use crate::object_permissions::error::ObjectPermissionError;
use crate::team_access::error::TeamAccessError;

// the failures the reassignment helpers raise, each carrying the answer LiteLLM gives for it
#[derive(Debug, thiserror::Error)]
pub enum KeyReassignmentError {
    #[error(transparent)]
    KeyPatch(#[from] KeyPatchError),
    #[error("the key table is unavailable")]
    KeyTable(#[from] TokenRepositoryError<Infallible>),
    #[error("the key query failed")]
    KeyQuery(#[from] TokenTableError),
    #[error("a stored key row is invalid")]
    KeyDecode(#[from] TokenDecodeError),
    #[error(transparent)]
    ObjectPermission(#[from] ObjectPermissionError),
    #[error(transparent)]
    Budget(#[from] BudgetRepositoryError),
    #[error("Key not found. No key with key_alias='{alias}'.")]
    AliasNotFound { alias: String },
    #[error("Multiple keys share key_alias='{alias}', so it cannot be used as an identifier.")]
    AliasAmbiguous { alias: String },
    #[error("either key or key_alias must be provided")]
    MissingKeyIdentifier,
    #[error("Only proxy admins can set end_user_budget_id on a key.")]
    EndUserBudgetForbidden,
    #[error("end_user_budget_id={budget_id} does not match any budget.")]
    EndUserBudgetUnknown { budget_id: String },
    #[error("Key has a tpm_limit={key_tpm} which is greater than the team's tpm_limit={team_tpm}.")]
    TpmExceedsTeam { key_tpm: i64, team_tpm: i64 },
    #[error("Key has a rpm_limit={key_rpm} which is greater than the team's rpm_limit={team_rpm}.")]
    RpmExceedsTeam { key_rpm: i64, team_rpm: i64 },
    #[error(
        "User={user_id} is not a member of the team={team_id}. Check team members via `/team/info`."
    )]
    OwnerNotInTeam { user_id: String, team_id: String },
    #[error(
        "User={user_id} is not a Proxy Admin or Team Admin for team={team_id}. Please ask your Proxy Admin to allow this action under 'Member Permissions' for this team."
    )]
    InitiatorNotAdmin { user_id: String, team_id: String },
    #[error(transparent)]
    TeamAccess(#[from] TeamAccessError),
    #[error(transparent)]
    ModelAccess(#[from] EntityModelAccessError),
}

impl KeyReassignmentError {
    // the status and detail shape LiteLLM answers with; None is an exception the caller's handler maps
    #[must_use]
    pub fn response(&self) -> (Option<u16>, Option<DetailShape>) {
        match self {
            Self::KeyPatch(error) => error.response(),
            Self::AliasNotFound { .. } => (Some(404_u16), Some(DetailShape::Text)),
            Self::AliasAmbiguous { .. } | Self::MissingKeyIdentifier => {
                (Some(400_u16), Some(DetailShape::Text))
            }
            Self::EndUserBudgetForbidden => (Some(403_u16), Some(DetailShape::Error)),
            Self::EndUserBudgetUnknown { .. } => (Some(400_u16), Some(DetailShape::Error)),
            Self::TpmExceedsTeam { .. }
            | Self::RpmExceedsTeam { .. }
            | Self::OwnerNotInTeam { .. }
            | Self::InitiatorNotAdmin { .. } => (Some(403_u16), Some(DetailShape::Text)),
            Self::TeamAccess(error) => {
                let text_body = matches!(
                    error,
                    TeamAccessError::Denied
                        | TeamAccessError::PersonalKeyGroups { .. }
                        | TeamAccessError::GroupOptIn { .. }
                );
                (error.status(), text_body.then_some(DetailShape::Text))
            }
            Self::KeyTable(_)
            | Self::KeyQuery(_)
            | Self::KeyDecode(_)
            | Self::ObjectPermission(_)
            | Self::Budget(_) => (None, None),
            Self::ModelAccess(error) => {
                let status = error
                    .denial()
                    .and_then(|denial| denial.as_proxy_exception().code.parse::<u16>().ok());
                (status, None)
            }
        }
    }

    // a ProxyException keeps its own fields; the deny-by-member and model denials reach the caller this way
    #[must_use]
    pub fn proxy_exception(&self) -> Option<ProxyException> {
        if let Self::KeyPatch(error) = self {
            return error.proxy_exception();
        }
        if let Self::TeamAccess(error) = self {
            return error.proxy_exception();
        }
        if let Self::ModelAccess(error) = self {
            return error
                .denial()
                .map(|denial| denial.as_proxy_exception().clone());
        }
        let (error_type, param, code) = match self {
            Self::AliasNotFound { .. } => (ProxyErrorTypes::NotFoundError, "key_alias", 404),
            Self::AliasAmbiguous { .. } => (ProxyErrorTypes::BadRequestError, "key_alias", 400),
            Self::MissingKeyIdentifier => (ProxyErrorTypes::BadRequestError, "key", 400),
            // the three delegated variants already returned; naming them keeps the match exhaustive
            Self::KeyPatch(_)
            | Self::TeamAccess(_)
            | Self::ModelAccess(_)
            | Self::KeyTable(_)
            | Self::KeyQuery(_)
            | Self::KeyDecode(_)
            | Self::ObjectPermission(_)
            | Self::Budget(_)
            | Self::EndUserBudgetForbidden
            | Self::EndUserBudgetUnknown { .. }
            | Self::TpmExceedsTeam { .. }
            | Self::RpmExceedsTeam { .. }
            | Self::OwnerNotInTeam { .. }
            | Self::InitiatorNotAdmin { .. } => return None,
        };
        Some(raised(self.to_string(), error_type, param, code))
    }
}

fn raised(message: String, error_type: ProxyErrorTypes, param: &str, code: i64) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: error_type.to_string(),
        param: Some(param.to_owned()),
        code: ExceptionCode::Status(code),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
