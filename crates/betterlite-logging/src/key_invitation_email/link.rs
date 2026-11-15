use std::convert::Infallible;
use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;

use super::KeyInvitationEmailError;

// LiteLLM orders the user's invitations newest first and takes the first row
const FIND_INVITATION_SQL: &str = r#"SELECT "id" FROM "LiteLLM_InvitationLink" WHERE "user_id" = $1 ORDER BY "created_at" DESC LIMIT 1"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvitationId(pub String);

#[async_trait]
pub trait InvitationCreator: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn create_for_user(&self, user_id: &str) -> Result<InvitationId, Self::Error>;
}

#[must_use]
pub async fn construct_user_invitation_link<C>(
    user_id: Option<&str>,
    base_url: &str,
    database: Option<&PostgresStore>,
    creator: &C,
) -> String
where
    C: InvitationCreator,
{
    let (Some(user_id), Some(database)) = (user_id, database) else {
        return base_url.to_owned();
    };
    match find_invitation(database, user_id).await {
        Ok(Some(invitation_id)) => onboarding_link(base_url, &invitation_id),
        Ok(None) => match creator.create_for_user(user_id).await {
            Ok(invitation_id) => onboarding_link(base_url, &invitation_id),
            Err(error) => {
                tracing::error!("Error creating invitation link for user_id {user_id}: {error}");
                base_url.to_owned()
            }
        },
        Err(error) => {
            tracing::error!("Error creating invitation link for user_id {user_id}: {error}");
            base_url.to_owned()
        }
    }
}

async fn find_invitation(
    database: &PostgresStore,
    user_id: &str,
) -> Result<Option<InvitationId>, KeyInvitationEmailError> {
    let client = database.session().await?;
    let row = client
        .query_opt(FIND_INVITATION_SQL, &[&user_id])
        .await
        .map_err(backend::<Infallible>)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id: String = row.try_get(0).map_err(backend::<Infallible>)?;
    Ok(Some(InvitationId(id)))
}

fn onboarding_link(base_url: &str, invitation_id: &InvitationId) -> String {
    format!(
        "{}/ui/onboarding?invitation_id={}",
        base_url.trim_end_matches('/'),
        invitation_id.0
    )
}
