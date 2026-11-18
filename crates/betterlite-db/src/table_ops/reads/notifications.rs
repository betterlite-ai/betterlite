use crate::database_replica::client::DatabaseClient;

use crate::table_ops::access::{NOTIFICATION_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::{RowDecoder, UserNotificationRow};
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
        let rows = self
            .query_rows(
                NOTIFICATION_ATTRIBUTE,
                "find_unique",
                UNIQUE_SQL,
                &[&user_id],
                DATA_RETRY,
                OperationCallType::GetData,
                UserNotificationRow::decode_row,
            )
            .await?;
        Ok(rows.into_iter().next())
    }

    pub async fn user_notifications_all(
        &self,
    ) -> Result<Vec<UserNotificationRow>, TableOpsError<Client::Error>> {
        self.query_rows(
            NOTIFICATION_ATTRIBUTE,
            "find_many",
            ALL_SQL,
            &[],
            DATA_RETRY,
            OperationCallType::GetData,
            UserNotificationRow::decode_row,
        )
        .await
    }
}
