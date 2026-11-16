use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::http_analytics_loggers::delivery::{Delivery, DeliveryError};
use crate::supabase::client::SupabaseClient;

pub mod callback;
pub mod rows;
pub mod settings;
mod tests;

use rows::{InitiatedRow, RequestRow, TerminalRow};

pub const DEFAULT_TABLE_NAME: &str = "request_logs";
pub const ON_CONFLICT_COLUMN: &str = "litellm_call_id";
pub const DEFAULT_END_USER: &str = "default";
pub const UNKNOWN_MODEL_ERROR: &str = "Unable to map your input to a model.";

const INITIATED_STATUS: &str = "initiated";
const SUCCESS_STATUS: &str = "success";
const FAILURE_STATUS: &str = "failure";

#[derive(Debug, thiserror::Error)]
pub enum SupabaseLogError<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    Delivery(#[from] DeliveryError<E>),
    #[error("the request span overflows the range of a decimal")]
    ResponseTime,
}

pub struct SupabaseLogger<D: Delivery> {
    pub client: SupabaseClient<D>,
    pub table_name: String,
}

impl<D: Delivery> SupabaseLogger<D> {
    pub async fn input_log_event(
        &self,
        row: &InitiatedRow,
    ) -> Result<(), SupabaseLogError<D::Error>> {
        self.client.write(&self.table_name, row, None).await?;
        Ok(())
    }

    pub async fn log_event(&self, row: &RequestRow<'_>) -> Result<(), SupabaseLogError<D::Error>> {
        let response_time = row
            .end_time
            .checked_sub(row.start_time)
            .ok_or(SupabaseLogError::ResponseTime)?;
        let Some(terminal) = terminal_row(row, response_time) else {
            return Ok(());
        };
        self.client
            .write(&self.table_name, &terminal, Some(ON_CONFLICT_COLUMN))
            .await?;
        Ok(())
    }
}

// LiteLLM builds one dictionary per branch, so the keys a branch does not set stay absent
fn terminal_row<'row>(row: &RequestRow<'row>, response_time: Decimal) -> Option<TerminalRow<'row>> {
    let Json::Object(response) = row.response_obj else {
        tracing::warn!("the supabase response object is not a JSON object");
        return None;
    };
    let Some(model) = response.get("model") else {
        tracing::warn!("the supabase response object carries no model");
        return None;
    };
    let mut terminal = TerminalRow {
        response_time,
        model,
        total_cost: row.total_cost,
        messages: row.messages,
        response: None,
        error: None,
        end_user: row.end_user,
        litellm_call_id: row.litellm_call_id,
        status: SUCCESS_STATUS,
    };
    if let Some(choices) = response.get("choices") {
        terminal.response = Some(first_content(choices)?);
        return Some(terminal);
    }
    let Some(error @ Json::Text(text)) = response.get("error") else {
        return None;
    };
    if text.contains(UNKNOWN_MODEL_ERROR) {
        terminal.total_cost = Decimal::ZERO;
    }
    terminal.error = Some(error);
    terminal.status = FAILURE_STATUS;
    Some(terminal)
}

// LiteLLM reads response_obj["choices"][0]["message"]["content"] and lets a missing one raise into the catch
fn first_content(choices: &Json) -> Option<&Json> {
    let Json::List(entries) = choices else {
        return None;
    };
    let Json::Object(first) = entries.first()? else {
        return None;
    };
    let Json::Object(message) = first.get("message")? else {
        return None;
    };
    message.get("content")
}
