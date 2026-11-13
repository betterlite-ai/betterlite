pub mod common;
mod tests;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ProxyErrorTypes {
    BudgetExceeded,
    NoDbConnection,
    TokenNotFoundInDb,
    KeyModelAccessDenied,
    TeamModelAccessDenied,
    UserModelAccessDenied,
    OrgModelAccessDenied,
    ProjectModelAccessDenied,
    AgentModelAccessDenied,
    ModelCostMapMissing,
    ExpiredKey,
    AuthError,
    AuthProviderUnavailable,
    InternalServerError,
    BadRequestError,
    NotFoundError,
    ValidationError,
    CachePingError,
    TeamMemberPermissionError,
    KeyVectorStoreAccessDenied,
    TeamVectorStoreAccessDenied,
    OrgVectorStoreAccessDenied,
    TeamMemberAlreadyInTeam,
    ToolAccessDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAccessObject {
    Key,
    Team,
    User,
    Org,
    Project,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorStoreObject {
    Key,
    Team,
    Org,
}

impl ProxyErrorTypes {
    #[must_use]
    pub const fn model_access_denied_for(object: ModelAccessObject) -> Self {
        match object {
            ModelAccessObject::Key => Self::KeyModelAccessDenied,
            ModelAccessObject::Team => Self::TeamModelAccessDenied,
            ModelAccessObject::User => Self::UserModelAccessDenied,
            ModelAccessObject::Org => Self::OrgModelAccessDenied,
            ModelAccessObject::Project => Self::ProjectModelAccessDenied,
            ModelAccessObject::Agent => Self::AgentModelAccessDenied,
        }
    }

    #[must_use]
    pub const fn vector_store_access_denied_for(object: VectorStoreObject) -> Self {
        match object {
            VectorStoreObject::Key => Self::KeyVectorStoreAccessDenied,
            VectorStoreObject::Team => Self::TeamVectorStoreAccessDenied,
            VectorStoreObject::Org => Self::OrgVectorStoreAccessDenied,
        }
    }
}
