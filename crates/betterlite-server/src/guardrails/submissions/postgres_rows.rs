use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::bindings::GuardrailsTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::resource_repository::table::{
    KeyFilter, ResourceActions, ResourceBinding, RowWrite,
};
use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::{ConfigSyncTableActions, TableActions as _};

use super::rows::{NewSubmission, SubmissionRow, SubmissionRows, SubmissionRowsError, TeamFilter};

const KEY: &str = "guardrail_id";
const NAME: &str = "guardrail_name";
const PARAMS: &str = "litellm_params";
const INFO: &str = "guardrail_info";
const STATUS: &str = "status";
const TEAM_ID: &str = "team_id";
const SUBMITTED_AT: &str = "submitted_at";
const CREATED_AT: &str = "created_at";
const UPDATED_AT: &str = "updated_at";

pub struct SubmissionRowsBinding;

impl TableBinding for SubmissionRowsBinding {
    const NAME: &'static str = GuardrailsTable::NAME;
    const TABLE: &'static str = GuardrailsTable::TABLE;
}

// Prisma fills guardrail_id from @default(uuid()) in its client, so B726 generates it here
impl ResourceBinding for SubmissionRowsBinding {
    const KEY: &'static str = KEY;
    const ID: Option<&'static str> = Some(KEY);
    const UPDATED_AT: Option<&'static str> = Some(UPDATED_AT);
    type Create = RowWrite;
    type Update = RowWrite;
}

pub struct PostgresSubmissionRows<'db> {
    pub client: Option<PrismaClient<'db>>,
    pub publish: PublishChange<TableError>,
}

impl<'db> PostgresSubmissionRows<'db> {
    fn table(
        &self,
    ) -> Result<
        ConfigSyncTableActions<ResourceActions<'db, SubmissionRowsBinding>>,
        SubmissionRowsError,
    > {
        let repository = PrismaTableRepository::new(
            self.client,
            Arc::clone(&self.publish),
            SubmissionRowsBinding,
        );
        let actions = repository
            .with_writer()
            .table(ResourceActions::<SubmissionRowsBinding>::new)?;
        Ok(actions)
    }
}

impl SubmissionRows for PostgresSubmissionRows<'_> {
    async fn find_by_id(
        &self,
        guardrail_id: &str,
    ) -> Result<Option<SubmissionRow>, SubmissionRowsError> {
        let actions = self.table()?;
        let rows = actions
            .as_ref()
            .find::<SubmissionRow>(KeyFilter::Equal(guardrail_id))
            .await?;
        Ok(rows.into_iter().next())
    }

    async fn find_by_name(
        &self,
        guardrail_name: &str,
    ) -> Result<Option<SubmissionRow>, SubmissionRowsError> {
        let actions = self.table()?;
        let rows = actions
            .as_ref()
            .find::<SubmissionRow>(KeyFilter::All)
            .await?;
        Ok(rows
            .into_iter()
            .find(|row| row.guardrail_name == guardrail_name))
    }

    async fn find_team_rows(
        &self,
        filter: TeamFilter,
    ) -> Result<Vec<SubmissionRow>, SubmissionRowsError> {
        let actions = self.table()?;
        let rows = actions
            .as_ref()
            .find::<SubmissionRow>(KeyFilter::All)
            .await?;
        let mut kept: Vec<SubmissionRow> = rows
            .into_iter()
            .filter(|row| visible(row, &filter))
            .collect();
        kept.sort_by_key(|row| Reverse(row.created_at.map(Timestamp::utc)));
        Ok(kept)
    }

    async fn create(
        &self,
        submission: NewSubmission,
    ) -> Result<SubmissionRow, SubmissionRowsError> {
        let actions = self.table()?;
        let row = actions
            .create(RowWrite(submission_record(&submission)))
            .await?;
        let value =
            serde_json::to_value(row).map_err(|source| SubmissionRowsError::Row { source })?;
        serde_json::from_value(value).map_err(|source| SubmissionRowsError::Row { source })
    }
}

// _find_team_guardrail_rows' where clause: team_id not null, or team_id in the caller's teams
fn visible(row: &SubmissionRow, filter: &TeamFilter) -> bool {
    match filter {
        TeamFilter::AnyTeam => row.team_id.is_some(),
        TeamFilter::Teams(teams) => row
            .team_id
            .as_ref()
            .is_some_and(|team| teams.contains(team)),
    }
}

fn submission_record(submission: &NewSubmission) -> BTreeMap<String, JsonData> {
    let mut record = BTreeMap::new();
    record.insert(
        NAME.to_owned(),
        JsonData::String(submission.guardrail_name.clone()),
    );
    record.insert(
        PARAMS.to_owned(),
        JsonData::String(submission.litellm_params.clone()),
    );
    record.insert(
        INFO.to_owned(),
        JsonData::String(submission.guardrail_info.clone()),
    );
    record.insert(
        STATUS.to_owned(),
        JsonData::String(submission.status.clone()),
    );
    record.insert(
        TEAM_ID.to_owned(),
        JsonData::String(submission.team_id.clone()),
    );
    record.insert(
        SUBMITTED_AT.to_owned(),
        JsonData::String(submission.submitted_at.to_rfc3339()),
    );
    record.insert(
        CREATED_AT.to_owned(),
        JsonData::String(submission.created_at.to_rfc3339()),
    );
    record.insert(
        UPDATED_AT.to_owned(),
        JsonData::String(submission.updated_at.to_rfc3339()),
    );
    record
}
