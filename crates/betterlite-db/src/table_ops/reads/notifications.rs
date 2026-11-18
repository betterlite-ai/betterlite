use crate::database_replica::client::DatabaseClient;
use crate::table_ops::access::ReadQuery;
use crate::table_ops::access::TableAccess as _;
use std::future::Future;

use crate::table_ops::access::{NOTIFICATION_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::{RowDecoder as _, UserNotificationRow};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const UNIQUE_SQL: &str = "SELECT row_to_json(n)::text FROM \"LiteLLM_UserNotifications\" n WHERE n.\"user_id\" = $1 LIMIT 1";
const ALL_SQL: &str = "SELECT row_to_json(n)::text FROM \"LiteLLM_UserNotifications\" n";

pub trait NotificationReads<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn user_notification_unique(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<UserNotificationRow>, TableOpsError<Client::Error>>> + Send;
    fn user_notifications_all(
        &self,
    ) -> impl Future<Output = Result<Vec<UserNotificationRow>, TableOpsError<Client::Error>>> + Send;
}

impl<Client> NotificationReads<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn user_notification_unique(
        &self,
        user_id: &str,
    ) -> Result<Option<UserNotificationRow>, TableOpsError<Client::Error>> {
        let rows = self
            .query_rows(
                ReadQuery {
                    attribute: NOTIFICATION_ATTRIBUTE,
                    method: "find_unique",
                    sql: UNIQUE_SQL,
                    parameters: &[&user_id],
                },
                DATA_RETRY,
                OperationCallType::GetData,
                UserNotificationRow::decode_row,
            )
            .await?;
        Ok(rows.into_iter().next())
    }

    async fn user_notifications_all(
        &self,
    ) -> Result<Vec<UserNotificationRow>, TableOpsError<Client::Error>> {
        self.query_rows(
            ReadQuery {
                attribute: NOTIFICATION_ATTRIBUTE,
                method: "find_many",
                sql: ALL_SQL,
                parameters: &[],
            },
            DATA_RETRY,
            OperationCallType::GetData,
            UserNotificationRow::decode_row,
        )
        .await
    }
}
