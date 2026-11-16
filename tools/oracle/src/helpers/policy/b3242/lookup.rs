use std::future::{Future, ready};

use betterlite_policy::lookup::get_policy_by_id_from_db;
use betterlite_policy::row_adapter::{
    PolicyClause, PolicyOrder, PolicyRow, PolicyTable, PolicyTableSource,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Builder;

use super::helper_error;
use betterlite_oracle_helper_api::helper::HelperResult;

#[derive(Debug, Deserialize)]
pub(crate) struct Call {
    policy_id: String,
    #[serde(default)]
    row: Option<PolicyRow>,
    #[serde(default)]
    failure: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct DatabaseFailure(String);

#[derive(Clone)]
struct FakeTable {
    row: Option<PolicyRow>,
    failure: Option<String>,
}

impl FakeTable {
    fn stored(&self) -> Result<PolicyRow, DatabaseFailure> {
        self.row
            .clone()
            .ok_or_else(|| DatabaseFailure("no row".to_owned()))
    }
}

impl PolicyTable<PolicyRow> for FakeTable {
    type Failure = DatabaseFailure;

    fn create(
        &self,
        _data: PolicyClause,
    ) -> impl Future<Output = Result<PolicyRow, DatabaseFailure>> + Send {
        ready(self.stored())
    }

    fn find_unique(
        &self,
        _filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<PolicyRow>, DatabaseFailure>> + Send {
        let answer = self.failure.as_ref().map_or_else(
            || Ok(self.row.clone()),
            |message| Err(DatabaseFailure(message.clone())),
        );
        ready(answer)
    }

    fn find_many(
        &self,
        _filter: Option<PolicyClause>,
        _order: Option<PolicyOrder>,
    ) -> impl Future<Output = Result<Vec<PolicyRow>, DatabaseFailure>> + Send {
        ready(Ok(self.row.clone().into_iter().collect()))
    }

    fn update(
        &self,
        _filter: PolicyClause,
        _data: PolicyClause,
    ) -> impl Future<Output = Result<PolicyRow, DatabaseFailure>> + Send {
        ready(self.stored())
    }

    fn update_many(
        &self,
        _filter: PolicyClause,
        _data: PolicyClause,
    ) -> impl Future<Output = Result<usize, DatabaseFailure>> + Send {
        ready(Ok(0))
    }

    fn delete(
        &self,
        _filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<PolicyRow>, DatabaseFailure>> + Send {
        ready(Ok(self.row.clone()))
    }

    fn delete_many(
        &self,
        _filter: PolicyClause,
    ) -> impl Future<Output = Result<usize, DatabaseFailure>> + Send {
        ready(Ok(0))
    }
}

struct FakeSource {
    table: FakeTable,
}

impl PolicyTableSource for FakeSource {
    type Table = FakeTable;

    fn policy_table(&self) -> Self::Table {
        self.table.clone()
    }
}

pub(super) fn run(call: &Call) -> HelperResult {
    let source = FakeSource {
        table: FakeTable {
            row: call.row.clone(),
            failure: call.failure.clone(),
        },
    };
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("OSError", error.to_string()))?;
    match runtime.block_on(get_policy_by_id_from_db(&call.policy_id, &source)) {
        Ok(None) => Ok(Value::Null),
        Ok(Some(response)) => serde_json::to_value(response)
            .map_err(|error| helper_error("ValueError", error.to_string())),
        Err(error) => Err(helper_error("PolicyLookupError", error.to_string())),
    }
}
