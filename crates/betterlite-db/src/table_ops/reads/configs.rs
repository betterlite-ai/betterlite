use crate::database_replica::client::DatabaseClient;
use crate::table_ops::access::TableAccess as _;
use std::future::Future;

use crate::table_ops::access::{CONFIG_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::{ConfigParamRow, RowDecoder as _};
use crate::table_ops::{TableOps, TableOpsError};

const NAMED_SQL: &str = "SELECT row_to_json(c)::text FROM \"LiteLLM_Config\" c WHERE c.\"param_name\" = ANY($1::text[])";

pub trait ConfigReads<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn configs_named(
        &self,
        param_names: &[String],
    ) -> impl Future<Output = Result<Vec<ConfigParamRow>, TableOpsError<Client::Error>>> + Send;
}

impl<Client> ConfigReads<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    // LiteLLM's prefetch calls find_many directly, outside get_data, so this read has no retry
    async fn configs_named(
        &self,
        param_names: &[String],
    ) -> Result<Vec<ConfigParamRow>, TableOpsError<Client::Error>> {
        let texts = self
            .text_rows(CONFIG_ATTRIBUTE, "find_many", NAMED_SQL, &[&param_names])
            .await?;
        let mut configs = Vec::with_capacity(texts.len());
        for text in &texts {
            configs.push(ConfigParamRow::decode_row(text)?);
        }
        Ok(configs)
    }
}
