use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::key_repository::decode::decode_mapping;
use betterlite_db::key_user_writes::hash::hash_token_if_needed;
use betterlite_db::key_user_writes::json_text::{SerdeJsonEncoder, jsonify_object};
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::DbSession;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::convert::Infallible;
use tokio_postgres::Row as StoredRow;

use crate::key_fields::decode::to_object;
use crate::key_fields::shared::with_settings_updated_at;
use crate::key_policy_fields::{microsecond_precision, zulu_datetime};
use crate::management_guards::amounts::Amount;

use super::Row;
use super::error::KeyPatchError;
use super::soft_budget::{SoftBudgetStore, apply_soft_budget_update};

const UPDATE_BUDGET_SQL: &str = "UPDATE \"LiteLLM_BudgetTable\" SET \"soft_budget\" = $1::text::double precision, \"updated_by\" = $2::text, \"updated_at\" = now() WHERE \"budget_id\" = $3::text";
const INSERT_BUDGET_SQL: &str = "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"soft_budget\", \"created_by\", \"updated_by\") VALUES ($1::text, $2::text::double precision, $3::text, $3::text)";
const SELECT_PERMISSION_SQL: &str = "SELECT row_to_json(p)::text FROM \"LiteLLM_ObjectPermissionTable\" AS p WHERE p.\"object_permission_id\" = $1::text";

pub struct KeyPatchWrite<'write> {
    pub key: &'write str,
    pub soft_budget: Option<Amount>,
    pub values: Row,
    pub existing_budget_id: Option<&'write str>,
    pub changed_by: &'write str,
    pub now: DateTime<Utc>,
}

pub struct KeyPatchOutcome {
    pub token: String,
    pub data: Row,
}

// ast-grep-ignore: no-long-names - _update_key_row_with_soft_budget: the budget row and then the key row, in one transaction.
pub async fn update_key_row_with_soft_budget(
    prisma: PrismaClient<'_>,
    write: &KeyPatchWrite<'_>,
) -> Result<KeyPatchOutcome, KeyPatchError> {
    let hashed = hash_token_if_needed(write.key);
    let mut session = prisma.writer.session().await?;
    let transaction = session.transaction().await.map_err(database_failure)?;
    let store = TransactionBudgetStore {
        transaction: &transaction,
    };
    let values = apply_soft_budget_update(
        &store,
        write.soft_budget,
        write.values.clone(),
        write.existing_budget_id,
        write.changed_by,
    )
    .await?;
    let data = write_key_row(&transaction, &hashed, values, write.now).await?;
    transaction.commit().await.map_err(database_failure)?;
    Ok(KeyPatchOutcome {
        token: hashed,
        data,
    })
}

// ast-grep-ignore: no-one-line-functions - one Postgres error conversion for every statement in the patch transaction
fn database_failure(source: tokio_postgres::Error) -> KeyPatchError {
    KeyPatchError::Database(backend::<Infallible>(source))
}

struct TransactionBudgetStore<'borrow, 'session> {
    transaction: &'borrow DbSession<'session>,
}

#[async_trait]
impl SoftBudgetStore for TransactionBudgetStore<'_, '_> {
    async fn update_soft_budget(
        &self,
        budget_id: &str,
        soft_budget: Option<Decimal>,
        updated_by: &str,
    ) -> Result<(), KeyPatchError> {
        let amount = soft_budget.map(|value| value.to_string());
        self.transaction
            .execute(UPDATE_BUDGET_SQL, &[&amount, &updated_by, &budget_id])
            .await
            .map(drop)
            .map_err(database_failure)
    }

    async fn create_soft_budget(
        &self,
        soft_budget: Decimal,
        created_by: &str,
    ) -> Result<String, KeyPatchError> {
        let budget_id = uuid::Uuid::new_v4().to_string();
        let amount = soft_budget.to_string();
        self.transaction
            .execute(INSERT_BUDGET_SQL, &[&budget_id, &amount, &created_by])
            .await
            .map_err(database_failure)?;
        Ok(budget_id)
    }
}

// The settings stamp and the updated_at Prisma fills in, then one statement for every patched column.
async fn write_key_row(
    transaction: &DbSession<'_>,
    hashed: &str,
    values: Row,
    now: DateTime<Utc>,
) -> Result<Row, KeyPatchError> {
    let mut record = with_settings_updated_at(&values, now);
    record.insert(String::from("token"), JsonData::String(hashed.to_owned()));
    record
        .entry(String::from("updated_at"))
        .or_insert_with(|| JsonData::String(zulu_datetime(microsecond_precision(now))));
    let converted = jsonify_object(&record, &SerdeJsonEncoder);
    let statement = update_statement(&converted)?;
    let json = serde_json::to_string(&JsonData::Object(converted))?;
    let stored = transaction
        .query_opt(statement.as_str(), &[&json, &hashed])
        .await
        .map_err(database_failure)?;
    let Some(stored) = stored else {
        return Ok(Row::new());
    };
    let mut fields = row_object(&stored)?;
    attach_object_permission(transaction, &mut fields).await?;
    // the row dump the source returns: the stored row decoded to the key model
    let model: LiteLLMVerificationToken = decode_mapping(fields)?;
    Ok(to_object(&model)?)
}

// The row type is the column list: jsonb_populate_record casts each JSON value to its column's type.
fn update_statement(fields: &Row) -> Result<String, KeyPatchError> {
    let mut assignments = Vec::with_capacity(fields.len());
    for name in fields.keys() {
        if !is_column_name(name) {
            return Err(KeyPatchError::UnknownColumn { name: name.clone() });
        }
        assignments.push(format!("\"{name}\" = r.\"{name}\""));
    }
    Ok(format!(
        "UPDATE \"LiteLLM_VerificationToken\" AS t SET {} FROM jsonb_populate_record(NULL::\"LiteLLM_VerificationToken\", $1::text::jsonb) AS r WHERE t.\"token\" = $2::text RETURNING row_to_json(t)::text",
        assignments.join(", ")
    ))
}

fn is_column_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

// include: {object_permission: true} leaves the relation null when the row names no permission
async fn attach_object_permission(
    transaction: &DbSession<'_>,
    fields: &mut Row,
) -> Result<(), KeyPatchError> {
    let permission_id = match fields.get("object_permission_id") {
        Some(JsonData::String(id)) => Some(id.clone()),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    };
    let value = match permission_id {
        Some(id) => {
            let stored = transaction
                .query_opt(SELECT_PERMISSION_SQL, &[&id])
                .await
                .map_err(database_failure)?;
            match stored {
                Some(row) => row_json(&row)?,
                None => JsonData::Null,
            }
        }
        None => JsonData::Null,
    };
    fields.insert(String::from("object_permission"), value);
    Ok(())
}

fn row_object(row: &StoredRow) -> Result<Row, KeyPatchError> {
    match row_json(row)? {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(KeyPatchError::StoredRowShape),
    }
}

fn row_json(row: &StoredRow) -> Result<JsonData, KeyPatchError> {
    let text: String = row.try_get(0).map_err(database_failure)?;
    let parsed = serde_json::from_str(&text)?;
    Ok(parsed)
}
