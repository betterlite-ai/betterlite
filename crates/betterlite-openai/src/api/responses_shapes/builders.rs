use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{CodeInterpreterLogType, OutputCodeInterpreterCallLog};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchSource {
    pub r#type: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchAction {
    pub r#type: String,
    pub query: String,
    pub queries: Vec<String>,
    pub sources: Vec<WebSearchSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchCall {
    pub id: String,
    pub r#type: String,
    pub status: String,
    pub action: WebSearchAction,
}

pub fn build_web_search_call(
    tool_id: &str,
    tool_input: &Value,
    result: &Value,
    status: Option<&str>,
) -> WebSearchCall {
    let query = tool_input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("");
    let content = result.get("content");
    let sources = content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let found = item.get("type").and_then(Value::as_str) == Some("web_search_result");
            let url = item.get("url").and_then(Value::as_str);
            (found).then_some(url).flatten().map(|url| WebSearchSource {
                r#type: "url".to_owned(),
                url: url.to_owned(),
            })
        })
        .collect();
    let failed = content
        .and_then(Value::as_object)
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str)
        == Some("web_search_tool_result_error");
    let selected_status = status.unwrap_or(if failed { "failed" } else { "completed" });
    WebSearchCall {
        id: format!("ws_{tool_id}"),
        r#type: "web_search_call".to_owned(),
        status: selected_status.to_owned(),
        action: WebSearchAction {
            r#type: "search".to_owned(),
            query: query.to_owned(),
            queries: if query.is_empty() {
                Vec::new()
            } else {
                vec![query.to_owned()]
            },
            sources,
        },
    }
}

pub fn build_interpreter_logs(content: &Value) -> Option<Vec<OutputCodeInterpreterCallLog>> {
    let object = content.as_object()?;
    let mut logs = String::new();
    if let Some(stdout) = object.get("stdout").and_then(Value::as_str) {
        logs.push_str(stdout);
    }
    if let Some(stderr) = object
        .get("stderr")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        logs.push_str("STDERR: ");
        logs.push_str(stderr);
    }
    if logs.is_empty() {
        None
    } else {
        Some(vec![OutputCodeInterpreterCallLog {
            r#type: CodeInterpreterLogType::Logs,
            logs,
        }])
    }
}
