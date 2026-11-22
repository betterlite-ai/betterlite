use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::model_group_data::errors::GroupDataError;
use crate::model_group_data::ports::{
    BudgetInclude, GroupBudgetCreate, GroupBudgetUpdate, GroupDatabase,
};
use crate::model_group_data::types::{BudgetRow, DeploymentRow, GroupBudgetRow};

struct ScriptedState {
    deployments: Vec<DeploymentRow>,
    budget_rows: Vec<BudgetRow>,
    budgets: Vec<GroupBudgetRow>,
    updates: Vec<(String, String)>,
    update_calls: usize,
    delete_calls: Vec<String>,
    budget_reads: usize,
}

// Stands in for the two Prisma tables, keeping insertion order where Postgres returns it.
pub(crate) struct ScriptedDatabase {
    state: Mutex<ScriptedState>,
    pub(crate) fail_update_at: Option<usize>,
    pub(crate) fail_delete: bool,
}

impl ScriptedDatabase {
    pub(crate) fn new(
        deployments: Vec<DeploymentRow>,
        budget_rows: Vec<BudgetRow>,
        budgets: Vec<GroupBudgetRow>,
    ) -> Self {
        Self {
            state: Mutex::new(ScriptedState {
                deployments,
                budget_rows,
                budgets,
                updates: Vec::new(),
                update_calls: 0,
                delete_calls: Vec::new(),
                budget_reads: 0,
            }),
            fail_update_at: None,
            fail_delete: false,
        }
    }

    pub(crate) fn updates(&self) -> Vec<(String, String)> {
        let state = self.state.lock().expect("scripted database");
        state.updates.clone()
    }

    pub(crate) fn delete_calls(&self) -> Vec<String> {
        let state = self.state.lock().expect("scripted database");
        state.delete_calls.clone()
    }

    pub(crate) fn budget(&self, access_group: &str) -> Option<GroupBudgetRow> {
        let state = self.state.lock().expect("scripted database");
        state
            .budgets
            .iter()
            .find(|row| row.access_group_name == access_group)
            .cloned()
    }

    pub(crate) fn budget_reads(&self) -> usize {
        let state = self.state.lock().expect("scripted database");
        state.budget_reads
    }

    pub(crate) fn has_budget_row(&self, budget_id: &str) -> bool {
        let state = self.state.lock().expect("scripted database");
        state
            .budget_rows
            .iter()
            .any(|row| row.budget_id == budget_id)
    }

    pub(crate) fn deployment_info(&self, model_id: &str) -> Option<Json> {
        let state = self.state.lock().expect("scripted database");
        state
            .deployments
            .iter()
            .find(|row| row.model_id == model_id)
            .and_then(|row| row.model_info.clone())
    }

    fn deployments_matching(&self, model_names: Option<&[String]>) -> Vec<DeploymentRow> {
        let state = self.state.lock().expect("scripted database");
        state
            .deployments
            .iter()
            .filter(|row| {
                model_names.is_none_or(|names| names.iter().any(|name| name == &row.model_name))
            })
            .cloned()
            .collect()
    }

    fn deployment_by_id(&self, model_id: &str) -> Option<DeploymentRow> {
        let state = self.state.lock().expect("scripted database");
        state
            .deployments
            .iter()
            .find(|row| row.model_id == model_id)
            .cloned()
    }

    fn budget_rows(&self, include: BudgetInclude) -> Vec<GroupBudgetRow> {
        let mut state = self.state.lock().expect("scripted database");
        state.budget_reads = state.budget_reads.checked_add(1).expect("budget reads");
        state
            .budgets
            .iter()
            .map(|row| row_with_include(row, &state.budget_rows, include))
            .collect()
    }
}

fn joined_budget(budget_rows: &[BudgetRow], budget_id: Option<&str>) -> Option<BudgetRow> {
    let id = budget_id?;
    budget_rows.iter().find(|row| row.budget_id == id).cloned()
}

impl GroupDatabase for ScriptedDatabase {
    fn deployment_find_many(
        &self,
        model_names: Option<&[String]>,
    ) -> impl Future<Output = Result<Vec<DeploymentRow>, GroupDataError>> + Send {
        ready(Ok(self.deployments_matching(model_names)))
    }

    fn deployment_find_unique(
        &self,
        model_id: &str,
    ) -> impl Future<Output = Result<Option<DeploymentRow>, GroupDataError>> + Send {
        ready(Ok(self.deployment_by_id(model_id)))
    }

    fn update_model_info(
        &self,
        model_id: &str,
        model_info: &str,
    ) -> impl Future<Output = Result<(), GroupDataError>> + Send {
        let mut state = self.state.lock().expect("scripted database");
        let attempt = state.update_calls;
        state.update_calls = attempt.checked_add(1).expect("update calls");
        if self.fail_update_at == Some(attempt) {
            return ready(Err(GroupDataError::ValueError {
                message: "scripted update failure".to_owned(),
            }));
        }
        state
            .updates
            .push((model_id.to_owned(), model_info.to_owned()));
        let parsed = serde_json::from_str::<Json>(model_info).expect("scripted update JSON");
        if let Some(row) = state
            .deployments
            .iter_mut()
            .find(|row| row.model_id == model_id)
        {
            row.model_info = Some(parsed);
        }
        drop(state);
        ready(Ok(()))
    }

    fn budget_find_unique(
        &self,
        access_group: &str,
        include: BudgetInclude,
    ) -> impl Future<Output = Result<Option<GroupBudgetRow>, GroupDataError>> + Send {
        ready(Ok(self
            .budget_rows(include)
            .into_iter()
            .find(|row| row.access_group_name == access_group)))
    }

    fn budget_find_many(
        &self,
        include: BudgetInclude,
    ) -> impl Future<Output = Result<Vec<GroupBudgetRow>, GroupDataError>> + Send {
        ready(Ok(self.budget_rows(include)))
    }

    fn budget_upsert(
        &self,
        create: &GroupBudgetCreate,
        update: &GroupBudgetUpdate,
        include: BudgetInclude,
    ) -> impl Future<Output = Result<GroupBudgetRow, GroupDataError>> + Send {
        let mut state = self.state.lock().expect("scripted database");
        let existing = state
            .budgets
            .iter()
            .position(|row| row.access_group_name == create.access_group_name);
        if let Some(index) = existing {
            let joined = joined_budget(&state.budget_rows, update.budget_id.as_deref());
            state.budgets[index].budget_id = update.budget_id.clone();
            state.budgets[index].litellm_budget_table = joined;
            return ready(Ok(row_with_include(
                &state.budgets[index],
                &state.budget_rows,
                include,
            )));
        }
        let created = GroupBudgetRow {
            access_group_name: create.access_group_name.clone(),
            spend: Some(Decimal::ZERO),
            budget_id: create.budget_id.clone(),
            litellm_budget_table: joined_budget(&state.budget_rows, create.budget_id.as_deref()),
        };
        state.budgets.push(created.clone());
        ready(Ok(row_with_include(&created, &state.budget_rows, include)))
    }

    fn budget_delete(
        &self,
        access_group: &str,
    ) -> impl Future<Output = Result<Option<GroupBudgetRow>, GroupDataError>> + Send {
        let mut state = self.state.lock().expect("scripted database");
        state.delete_calls.push(access_group.to_owned());
        if self.fail_delete {
            return ready(Err(GroupDataError::ValueError {
                message: "scripted delete failure".to_owned(),
            }));
        }
        let position = state
            .budgets
            .iter()
            .position(|row| row.access_group_name == access_group);
        let removed = position.map(|index| state.budgets.remove(index));
        drop(state);
        ready(Ok(removed))
    }
}

fn row_with_include(
    row: &GroupBudgetRow,
    budget_rows: &[BudgetRow],
    include: BudgetInclude,
) -> GroupBudgetRow {
    let mut found = row.clone();
    found.litellm_budget_table = if include.litellm_budget_table {
        joined_budget(budget_rows, row.budget_id.as_deref())
    } else {
        None
    };
    found
}
