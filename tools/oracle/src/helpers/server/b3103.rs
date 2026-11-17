use betterlite_analytics::daily_metrics::DailySpendPage;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::usage_ai_chat::fetch_args::{parse_csv_ids, resolve_fetch_kwargs};
use betterlite_server::usage_ai_chat::prompt::build_system_prompt;
use betterlite_server::usage_ai_chat::sse::{SseEvent, sse_frame};
use betterlite_server::usage_ai_chat::summary::{
    EntityLabel, summarise_entity_data, summarise_usage_data,
};
use betterlite_server::usage_ai_chat::tools::{UsageTool, tools_for_role};
use chrono::NaiveDate;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Tools {
        is_admin: bool,
    },
    Prompt {
        is_admin: bool,
        today: String,
    },
    CsvIds {
        raw: Option<String>,
    },
    FetchArgs {
        tool: UsageTool,
        arguments: IndexMap<String, String>,
        user_id: Option<String>,
        is_admin: bool,
    },
    SummaryUsage {
        page: DailySpendPage,
    },
    SummaryEntity {
        page: DailySpendPage,
        label: EntityLabel,
    },
    Sse {
        event: SseEvent,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Tools { is_admin } => Ok(json!({ "tools": tools_for_role(*is_admin) })),
            Input::Prompt { is_admin, today } => Ok(json!({
                "prompt": build_system_prompt(
                    *is_admin,
                    NaiveDate::parse_from_str(today, "%Y-%m-%d").or_value_error()?,
                )
            })),
            Input::CsvIds { raw } => Ok(json!({ "ids": parse_csv_ids(raw.as_deref()) })),
            Input::FetchArgs {
                tool,
                arguments,
                user_id,
                is_admin,
            } => {
                let fetch = resolve_fetch_kwargs(*tool, arguments, user_id.as_deref(), *is_admin)
                    .or_value_error()?;
                Ok(json!({ "kwargs": fetch }))
            }
            Input::SummaryUsage { page } => Ok(json!({
                "summary": summarise_usage_data(page).or_value_error()?
            })),
            Input::SummaryEntity { page, label } => Ok(json!({
                "summary": summarise_entity_data(page, *label).or_value_error()?
            })),
            Input::Sse { event } => Ok(json!({ "frame": sse_frame(event) })),
        }
    }
}

mod tests;
