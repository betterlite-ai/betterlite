use indexmap::IndexMap;
use serde::Serialize;

use super::error::UsageChatError;
use super::tools::UsageTool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FetchArguments {
    Global {
        start_date: String,
        end_date: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        user_id: Option<String>,
    },
    Team {
        start_date: String,
        end_date: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        team_ids: Option<String>,
    },
    Tag {
        start_date: String,
        end_date: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tags: Option<String>,
    },
}

#[must_use]
pub fn parse_csv_ids(raw: Option<&str>) -> Option<Vec<String>> {
    let text = raw.filter(|value| !value.is_empty())?;
    Some(
        text.split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

pub fn decode_tool_arguments(arguments: &str) -> Result<IndexMap<String, String>, UsageChatError> {
    serde_json::from_str(arguments)
        .map_err(|source| UsageChatError::InvalidToolArguments { source })
}

pub fn resolve_fetch_kwargs(
    tool: UsageTool,
    arguments: &IndexMap<String, String>,
    user_id: Option<&str>,
    is_admin: bool,
) -> Result<FetchArguments, UsageChatError> {
    let start_date = arguments.get("start_date").map_or("", String::as_str);
    let end_date = arguments.get("end_date").map_or("", String::as_str);
    if start_date.is_empty() || end_date.is_empty() {
        return Err(UsageChatError::MissingDates);
    }
    Ok(match tool {
        UsageTool::Usage => FetchArguments::Global {
            start_date: start_date.to_owned(),
            end_date: end_date.to_owned(),
            user_id: usage_user_id(arguments, user_id, is_admin)?,
        },
        UsageTool::Team => FetchArguments::Team {
            start_date: start_date.to_owned(),
            end_date: end_date.to_owned(),
            team_ids: non_empty(arguments, "team_ids"),
        },
        UsageTool::Tag => FetchArguments::Tag {
            start_date: start_date.to_owned(),
            end_date: end_date.to_owned(),
            tags: non_empty(arguments, "tags"),
        },
    })
}

fn usage_user_id(
    arguments: &IndexMap<String, String>,
    user_id: Option<&str>,
    is_admin: bool,
) -> Result<Option<String>, UsageChatError> {
    if !is_admin {
        let caller = user_id.ok_or(UsageChatError::UnscopedQuery)?;
        return Ok(Some(caller.to_owned()));
    }
    Ok(non_empty(arguments, "user_id"))
}

fn non_empty(arguments: &IndexMap<String, String>, key: &str) -> Option<String> {
    arguments
        .get(key)
        .filter(|value| !value.is_empty())
        .cloned()
}
