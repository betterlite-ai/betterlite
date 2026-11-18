use crate::database_replica::client::DatabaseClient;

use crate::table_ops::access::{NOTIFICATION_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::{RowError, UserNotificationRow, decode_model_rows};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const UNIQUE_SQL: &str = "SELECT row_to_json(n)::text FROM \"LiteLLM_UserNotifications\" n WHERE n.\"user_id\" = $1 LIMIT 1";
const ALL_SQL: &str = "SELECT row_to_json(n)::text FROM \"LiteLLM_UserNotifications\" n";

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn user_notification_unique(
        &self,
        user_id: &str,
    ) -> Result<Option<UserNotificationRow>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.user_notification_unique_once(user_id)
        })
        .await
    }

    pub async fn user_notifications_all(
        &self,
    ) -> Result<Vec<UserNotificationRow>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.user_notifications_all_once()
        })
        .await
    }

    async fn user_notification_unique_once(
        &self,
        user_id: &str,
    ) -> Result<Option<UserNotificationRow>, TableOpsError<Client::Error>> {
        let found = self
            .text_row(
                NOTIFICATION_ATTRIBUTE,
                "find_unique",
                UNIQUE_SQL,
                &[&user_id],
            )
            .await?;
        match found {
            Some(text) => Ok(Some(serde_json::from_str(&text).map_err(RowError::Model)?)),
            None => Ok(None),
        }
    }

    async fn user_notifications_all_once(
        &self,
    ) -> Result<Vec<UserNotificationRow>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(NOTIFICATION_ATTRIBUTE, "find_many", ALL_SQL, &[])
            .await?;
        decode_model_rows(&rows)
    }
}
