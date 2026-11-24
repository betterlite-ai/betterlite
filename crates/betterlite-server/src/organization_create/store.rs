use std::collections::BTreeMap;

use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::resource_repository::budget::{BudgetRepository, BudgetRepositoryError};
use betterlite_db::resource_repository::table::{ResourceActions, ResourceBinding, RowWrite};
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::TableActions;

// the LiteLLM_OrganizationTable binding B723's live tables use for the organization row
pub struct OrganizationBinding;

impl TableBinding for OrganizationBinding {
    const NAME: &'static str = "litellm_organizationtable";
    const TABLE: &'static str = "LiteLLM_OrganizationTable";
}

impl ResourceBinding for OrganizationBinding {
    const KEY: &'static str = "organization_id";
    const ID: Option<&'static str> = Some("organization_id");
    const UPDATED_AT: Option<&'static str> = Some("updated_at");
    type Create = RowWrite;
    type Update = RowWrite;
}

// The three child writes creation needs, over one Prisma client
pub struct OrganizationStore<'db> {
    pub client: PrismaClient<'db>,
}

impl<'db> OrganizationStore<'db> {
    pub async fn create_budget(
        &self,
        data: BTreeMap<String, JsonData>,
    ) -> Result<JsonData, TableError> {
        BudgetRepository::new(Some(self.client))
            .table()
            .map_err(TableError::Database)?
            .create(RowWrite(data))
            .await
    }

    pub async fn create_organization(
        &self,
        data: BTreeMap<String, JsonData>,
    ) -> Result<JsonData, TableError> {
        let publisher = redis_config_change_publisher(None);
        let repository =
            PrismaTableRepository::new(Some(self.client), publisher, OrganizationBinding);
        let actions = ResourceActions::<OrganizationBinding>::new;
        let table = repository.table(actions).map_err(TableError::Database)?;
        let write = RowWrite(data);
        TableActions::create(&table, write).await
    }

    pub async fn find_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, BudgetRepositoryError> {
        BudgetRepository::new(Some(self.client))
            .find_by_id(budget_id)
            .await
    }
}
