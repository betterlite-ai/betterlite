use betterlite_analytics::daily_metrics::DailySpendPage;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::usage_ai_chat::fetch_args::{
    FetchArguments, parse_csv_ids, resolve_fetch_kwargs,
};
use betterlite_server::usage_ai_chat::prompt::build_system_prompt;
use betterlite_server::usage_ai_chat::sse::{SseEvent, sse_frame};
use betterlite_server::usage_ai_chat::summary::{
    EntityLabel, summarise_entity_data, summarise_usage_data,
};
use betterlite_server::usage_ai_chat::tools::{UsageTool, tools_for_role};
use chrono::NaiveDate;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

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
        tool: ToolInput,
        arguments: IndexMap<String, String>,
        user_id: Option<String>,
        is_admin: bool,
    },
    SummaryUsage {
        page: DailySpendPage,
    },
    SummaryEntity {
        page: DailySpendPage,
        label: LabelInput,
    },
    Sse {
        event: SseEvent,
    },
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolInput {
    Usage,
    Team,
    Tag,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LabelInput {
    Team,
    Tag,
}

impl ToolInput {
    const fn tool(self) -> UsageTool {
        match self {
            Self::Usage => UsageTool::Usage,
            Self::Team => UsageTool::Team,
            Self::Tag => UsageTool::Tag,
        }
    }
}

impl LabelInput {
    const fn label(self) -> EntityLabel {
        match self {
            Self::Team => EntityLabel::Team,
            Self::Tag => EntityLabel::Tag,
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Tools { is_admin } => Ok(json!({ "tools": tools_for_role(*is_admin) })),
            Input::Prompt { is_admin, today } => Ok(json!({
                "prompt": build_system_prompt(*is_admin, day(today)?)
            })),
            Input::CsvIds { raw } => Ok(json!({ "ids": parse_csv_ids(raw.as_deref()) })),
            Input::FetchArgs {
                tool,
                arguments,
                user_id,
                is_admin,
            } => fetch_kwargs(*tool, arguments, user_id.as_deref(), *is_admin),
            Input::SummaryUsage { page } => Ok(json!({
                "summary": summarise_usage_data(page).or_value_error()?
            })),
            Input::SummaryEntity { page, label } => Ok(json!({
                "summary": summarise_entity_data(page, label.label()).or_value_error()?
            })),
            Input::Sse { event } => Ok(json!({ "frame": sse_frame(event) })),
        }
    }
}

fn day(text: &str) -> Result<NaiveDate, Box<HelperError>> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").or_value_error()
}

fn fetch_kwargs(
    tool: ToolInput,
    arguments: &IndexMap<String, String>,
    user_id: Option<&str>,
    is_admin: bool,
) -> HelperResult {
    let fetch = resolve_fetch_kwargs(tool.tool(), arguments, user_id, is_admin).or_value_error()?;
    Ok(json!({ "kwargs": kwargs(&fetch) }))
}

fn kwargs(fetch: &FetchArguments) -> Value {
    match fetch {
        FetchArguments::Global {
            start_date,
            end_date,
            user_id,
        } => match user_id {
            Some(id) => json!({
                "start_date": start_date,
                "end_date": end_date,
                "user_id": id,
            }),
            None => json!({ "start_date": start_date, "end_date": end_date }),
        },
        FetchArguments::Team {
            start_date,
            end_date,
            team_ids,
        } => match team_ids {
            Some(ids) => json!({
                "start_date": start_date,
                "end_date": end_date,
                "team_ids": ids,
            }),
            None => json!({ "start_date": start_date, "end_date": end_date }),
        },
        FetchArguments::Tag {
            start_date,
            end_date,
            tags,
        } => match tags {
            Some(names) => json!({
                "start_date": start_date,
                "end_date": end_date,
                "tags": names,
            }),
            None => json!({ "start_date": start_date, "end_date": end_date }),
        },
    }
}

mod tests;
