use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberIdentity {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MemberIdentityError {
    #[error("Either user id or user email must be provided")]
    MissingIdentity,
}

impl MemberIdentity {
    pub fn new(
        user_id: Option<String>,
        user_email: Option<String>,
    ) -> Result<Self, MemberIdentityError> {
        if user_id.is_none() && user_email.is_none() {
            return Err(MemberIdentityError::MissingIdentity);
        }
        Ok(Self {
            user_id,
            user_email,
        })
    }
}
