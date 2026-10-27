use serde_json::value::{Map, Value};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Result {
    pub inputs: Map<String, Value>,
    pub outputs: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub name: String,
    pub attributes: Value,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub span_kind: String,
    pub results: Vec<Result>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceTree {
    pub span: Span,
    pub model_dict: Value,
}

pub struct OpenAIRequestResponseResolver;

impl OpenAIRequestResponseResolver {
    #[must_use]
    pub fn resolve(request: &Value, response: &Value, time_elapsed: f64) -> Option<TraceTree> {
        let object = response.get("object")?.as_str()?;
        let results = match object {
            "edit" => Self::edit_results(request, response)?,
            "text_completion" => Self::completion_results(request, response)?,
            "chat.completion" => Self::chat_results(request, response)?,
            _ => return None,
        };
        Self::results_to_trace_tree(request, response, results, time_elapsed)
    }

    fn edit_results(request: &Value, response: &Value) -> Option<Vec<Result>> {
        let instruction = request.get("instruction")?;
        let input = request.get("input")?;
        let request_text = format!(
            "\n\n**Instruction**: {}\n\n**Input**: {}\n",
            display_value(instruction),
            display_value(input)
        );
        Self::text_results(response, "Edited", &request_text)
    }

    fn completion_results(request: &Value, response: &Value) -> Option<Vec<Result>> {
        let prompt = request.get("prompt")?;
        let request_text = format!("\n\n**Prompt**: {}\n", display_value(prompt));
        Self::text_results(response, "Completion", &request_text)
    }

    fn text_results(response: &Value, label: &str, request_text: &str) -> Option<Vec<Result>> {
        let choices = response.get("choices")?.as_array()?;
        Some(
            choices
                .iter()
                .filter_map(|choice| {
                    let text = choice.get("text")?;
                    Some(result(
                        request_text.to_owned(),
                        format!("\n\n**{label}**: {}\n", display_value(text)),
                    ))
                })
                .collect(),
        )
    }

    fn chat_results(request: &Value, response: &Value) -> Option<Vec<Result>> {
        let messages = request.get("messages")?.as_array()?;
        let request_text = messages
            .iter()
            .filter_map(|message| {
                Some(format!(
                    "\n\n**{}**: {}\n",
                    display_value(message.get("role")?),
                    display_value(message.get("content")?)
                ))
            })
            .collect::<String>();
        let choices = response.get("choices")?.as_array()?;
        Some(
            choices
                .iter()
                .filter_map(|choice| {
                    let message = choice.get("message")?;
                    Some(result(
                        request_text.clone(),
                        format!(
                            "\n\n**{}**: {}\n",
                            display_value(message.get("role")?),
                            display_value(message.get("content")?)
                        ),
                    ))
                })
                .collect(),
        )
    }

    fn results_to_trace_tree(
        request: &Value,
        response: &Value,
        results: Vec<Result>,
        time_elapsed: f64,
    ) -> Option<TraceTree> {
        let created = response.get("created")?.as_f64()?;
        let start_time_ms = seconds_to_millis(created)?;
        let elapsed_ms = seconds_to_millis(time_elapsed)?;
        let end_time_ms = start_time_ms.checked_add(elapsed_ms)?;
        let model = response
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("openai");
        let object = response.get("object")?.as_str()?;
        let created_text = response.get("created")?.to_string();
        let span = Span {
            name: format!("{model}_{object}_{created_text}"),
            attributes: response.clone(),
            start_time_ms,
            end_time_ms,
            span_kind: "LLM".to_owned(),
            results,
        };
        let mut model_dict = Map::new();
        model_dict.insert("request".to_owned(), request.clone());
        model_dict.insert("response".to_owned(), response.clone());
        model_dict.insert("_kind".to_owned(), Value::String("openai".to_owned()));
        Some(TraceTree {
            span,
            model_dict: Value::Object(model_dict),
        })
    }
}

fn result(request: String, response: String) -> Result {
    let mut inputs = Map::new();
    inputs.insert("request".to_owned(), Value::String(request));
    let mut outputs = Map::new();
    outputs.insert("response".to_owned(), Value::String(response));
    Result { inputs, outputs }
}

fn display_value(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn seconds_to_millis(seconds: f64) -> Option<i64> {
    let duration = Duration::try_from_secs_f64(seconds).ok()?;
    i64::try_from(duration.as_millis()).ok()
}
