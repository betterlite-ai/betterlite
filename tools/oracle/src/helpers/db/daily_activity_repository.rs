use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::daily_activity_contracts::{ExportRow, ExportType};
use betterlite_db::daily_activity_repository::USAGE_EXPORT_BATCH_SIZE;
use betterlite_db::daily_activity_repository::export::next_cursor;
use betterlite_db::daily_activity_repository::metadata::{metadata_row, metadata_tags};
use betterlite_db::daily_activity_repository::rows::VerificationTokenRow;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    cases: BTreeMap<String, Case>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Case {
    MetadataTags {
        value: Option<JsonData>,
    },
    MetadataRow {
        token: String,
        key_alias: Option<String>,
        team_id: Option<String>,
        user_id: Option<String>,
        metadata: Option<BTreeMap<String, JsonData>>,
        key_exists: bool,
    },
    NextCursor {
        export_type: ExportType,
        last: ExportRow,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let answers: BTreeMap<String, serde_json::Value> = input
            .cases
            .iter()
            .map(|(name, case)| (name.clone(), answer(case)))
            .collect();
        serde_json::to_value(answers).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn answer(case: &Case) -> serde_json::Value {
    match case {
        Case::MetadataTags { value } => {
            serde_json::json!({ "tags": metadata_tags(value.as_ref()) })
        }
        Case::MetadataRow {
            token,
            key_alias,
            team_id,
            user_id,
            metadata,
            key_exists,
        } => {
            let row = VerificationTokenRow {
                token: token.clone(),
                key_alias: key_alias.clone(),
                team_id: team_id.clone(),
                user_id: user_id.clone(),
                metadata: metadata.clone(),
            };
            serde_json::json!({ "row": metadata_row(&row, *key_exists) })
        }
        Case::NextCursor { export_type, last } => {
            serde_json::json!({ "cursor": next_cursor(&export_batch(last), *export_type) })
        }
    }
}

fn export_batch(last: &ExportRow) -> Vec<ExportRow> {
    let mut batch: Vec<ExportRow> = (0..USAGE_EXPORT_BATCH_SIZE.saturating_sub(1))
        .map(|_| default_export_row())
        .collect();
    batch.push(last.clone());
    batch
}

fn default_export_row() -> ExportRow {
    ExportRow {
        date: "2026-08-01".to_owned(),
        entity_id: "entity".to_owned(),
        entity_alias: None,
        api_key: None,
        key_alias: None,
        user_id: None,
        user_email: None,
        model: None,
        spend: Decimal::ZERO,
        flat_cost: Decimal::ZERO,
        prompt_tokens: 0,
        completion_tokens: 0,
        api_requests: 0,
        successful_requests: 0,
        failed_requests: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    }
}
