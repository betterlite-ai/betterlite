use std::sync::Mutex;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::model_group_data::errors::GroupDataError;
use crate::model_group_data::ports::{
    BudgetInclude, GroupBudgetCreate, GroupBudgetUpdate, GroupDatabase,
};
use crate::model_group_data::types::{BudgetRow, DeploymentRow, GroupBudgetRow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpsertCall {
    pub create_name: String,
    pub create_budget_id: Option<String>,
    pub create_created_by: String,
    pub create_updated_by: String,
    pub update_budget_id: Option<String>,
    pub update_updated_by: String,
}

pub struct ScriptedState {
    pub deployments: Vec<DeploymentRow>,
    pub budget_rows: Vec<BudgetRow>,
    pub budgets: Vec<GroupBudgetRow>,
    pub updates: Vec<(String, String)>,
    pub update_calls: usize,
    pub delete_calls: Vec<String>,
    pub upserts: Vec<UpsertCall>,
    pub budget_reads: usize,
}

// Stands in for the two Prisma tables, keeping insertion order where Postgres returns it.
pub struct ScriptedDatabase {
    pub state: Mutex<ScriptedState>,
    pub fail_update_at: Option<usize>,
    pub fail_delete: bool,
}

impl ScriptedDatabase {
    pub fn new(
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
                upserts: Vec::new(),
                budget_reads: 0,
            }),
            fail_update_at: None,
            fail_delete: false,
        }
    }

    pub fn updates(&self) -> Vec<(String, String)> {
        let state = self.state.lock().expect("scripted database");
        state.updates.clone()
    }

    pub fn delete_calls(&self) -> Vec<String> {
        let state = self.state.lock().expect("scripted database");
        state.delete_calls.clone()
    }

    pub fn upsert_calls(&self) -> Vec<UpsertCall> {
        let state = self.state.lock().expect("scripted database");
        state.upserts.clone()
    }

    pub fn budget(&self, access_group: &str) -> Option<GroupBudgetRow> {
        let state = self.state.lock().expect("scripted database");
        state
            .budgets
            .iter()
            .find(|row| row.access_group_name == access_group)
            .cloned()
    }

    pub fn budget_reads(&self) -> usize {
        let state = self.state.lock().expect("scripted database");
        state.budget_reads
    }

    pub fn has_budget_row(&self, budget_id: &str) -> bool {
        let state = self.state.lock().expect("scripted database");
        state
            .budget_rows
            .iter()
            .any(|row| row.budget_id == budget_id)
    }

    pub fn deployment_info(&self, model_id: &str) -> Option<Json> {
        let state = self.state.lock().expect("scripted database");
        state
            .deployments
            .iter()
            .find(|row| row.model_id == model_id)
            .and_then(|row| row.model_info.clone())
    }
}

fn joined_budget(budget_rows: &[BudgetRow], budget_id: &Option<String>) -> Option<BudgetRow> {
    let id = budget_id.as_ref()?;
    budget_rows.iter().find(|row| &row.budget_id == id).cloned()
}

impl GroupDatabase for ScriptedDatabase {
    async fn deployment_find_many(
        &self,
        model_names: Option<&[String]>,
    ) -> Result<Vec<DeploymentRow>, GroupDataError> {
        let state = self.state.lock().expect("scripted database");
        let found = state
            .deployments
            .iter()
            .filter(|row| {
                model_names.is_none_or(|names| names.iter().any(|name| name == &row.model_name))
            })
            .cloned()
            .collect();
        Ok(found)
    }

    async fn deployment_find_unique(
        &self,
        model_id: &str,
    ) -> Result<Option<DeploymentRow>, GroupDataError> {
        let state = self.state.lock().expect("scripted database");
        let found = state
            .deployments
            .iter()
            .find(|row| row.model_id == model_id)
            .cloned();
        Ok(found)
    }

    async fn update_model_info(
        &self,
        model_id: &str,
        model_info: &str,
    ) -> Result<(), GroupDataError> {
        let mut state = self.state.lock().expect("scripted database");
        let attempt = state.update_calls;
        state.update_calls = attempt.checked_add(1).expect("update calls");
        if self.fail_update_at == Some(attempt) {
            return Err(GroupDataError::ValueError {
                message: "scripted update failure".to_owned(),
            });
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
        Ok(())
    }

    async fn budget_find_unique(
        &self,
        access_group: &str,
        include: BudgetInclude,
    ) -> Result<Option<GroupBudgetRow>, GroupDataError> {
        let mut state = self.state.lock().expect("scripted database");
        state.budget_reads = state.budget_reads.checked_add(1).expect("budget reads");
        let found = state
            .budgets
            .iter()
            .find(|row| row.access_group_name == access_group)
            .map(|row| row_with_include(row, &state.budget_rows, include));
        Ok(found)
    }

    async fn budget_find_many(
        &self,
        include: BudgetInclude,
    ) -> Result<Vec<GroupBudgetRow>, GroupDataError> {
        let mut state = self.state.lock().expect("scripted database");
        state.budget_reads = state.budget_reads.checked_add(1).expect("budget reads");
        let found = state
            .budgets
            .iter()
            .map(|row| row_with_include(row, &state.budget_rows, include))
            .collect();
        Ok(found)
    }

    async fn budget_upsert(
        &self,
        create: &GroupBudgetCreate,
        update: &GroupBudgetUpdate,
        include: BudgetInclude,
    ) -> Result<GroupBudgetRow, GroupDataError> {
        let mut state = self.state.lock().expect("scripted database");
        state.upserts.push(UpsertCall {
            create_name: create.access_group_name.clone(),
            create_budget_id: create.budget_id.clone(),
            create_created_by: create.created_by.clone(),
            create_updated_by: create.updated_by.clone(),
            update_budget_id: update.budget_id.clone(),
            update_updated_by: update.updated_by.clone(),
        });
        let existing = state
            .budgets
            .iter()
            .position(|row| row.access_group_name == create.access_group_name);
        let row = match existing {
            Some(index) => {
                let joined = joined_budget(&state.budget_rows, &update.budget_id);
                state.budgets[index].budget_id = update.budget_id.clone();
                state.budgets[index].litellm_budget_table = joined;
                row_with_include(&state.budgets[index], &state.budget_rows, include)
            }
            None => {
                let row = GroupBudgetRow {
                    access_group_name: create.access_group_name.clone(),
                    spend: Some(Decimal::ZERO),
                    budget_id: create.budget_id.clone(),
                    litellm_budget_table: joined_budget(&state.budget_rows, &create.budget_id),
                };
                state.budgets.push(row.clone());
                row_with_include(&row, &state.budget_rows, include)
            }
        };
        Ok(row)
    }

    async fn budget_delete(
        &self,
        access_group: &str,
    ) -> Result<Option<GroupBudgetRow>, GroupDataError> {
        let mut state = self.state.lock().expect("scripted database");
        state.delete_calls.push(access_group.to_owned());
        if self.fail_delete {
            return Err(GroupDataError::ValueError {
                message: "scripted delete failure".to_owned(),
            });
        }
        let position = state
            .budgets
            .iter()
            .position(|row| row.access_group_name == access_group);
        Ok(position.map(|index| state.budgets.remove(index)))
    }
}

fn row_with_include(
    row: &GroupBudgetRow,
    budget_rows: &[BudgetRow],
    include: BudgetInclude,
) -> GroupBudgetRow {
    let mut found = row.clone();
    found.litellm_budget_table = include
        .litellm_budget_table
        .then(|| joined_budget(budget_rows, &row.budget_id))
        .flatten();
    found
}
