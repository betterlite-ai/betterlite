use betterlite_models::decisions::Json as Plain;
use betterlite_prompts::arguments::json::Json as Prompted;
use indexmap::IndexMap;
use serde_json::Value;

enum Source<'source> {
    Completion(&'source Value),
    Prompt(&'source Prompted),
}

pub fn completion_body(body: &IndexMap<String, Value>) -> IndexMap<String, Plain> {
    body.iter()
        .map(|(name, value)| (name.clone(), convert(Source::Completion(value))))
        .collect()
}

pub fn prompt_body(body: &IndexMap<String, Prompted>) -> IndexMap<String, Plain> {
    body.iter()
        .map(|(name, value)| (name.clone(), convert(Source::Prompt(value))))
        .collect()
}

fn convert(source: Source<'_>) -> Plain {
    match source {
        Source::Completion(Value::Null) | Source::Prompt(Prompted::Null) => Plain::Null,
        Source::Completion(Value::Bool(flag)) | Source::Prompt(Prompted::Bool(flag)) => {
            Plain::Bool(*flag)
        }
        Source::Completion(Value::Number(number)) | Source::Prompt(Prompted::Number(number)) => {
            Plain::Number(number.clone())
        }
        Source::Completion(Value::String(text)) | Source::Prompt(Prompted::Text(text)) => {
            Plain::Text(text.clone())
        }
        Source::Completion(Value::Array(items)) => Plain::List(
            items
                .iter()
                .map(|item| convert(Source::Completion(item)))
                .collect(),
        ),
        Source::Prompt(Prompted::List(items)) => Plain::List(
            items
                .iter()
                .map(|item| convert(Source::Prompt(item)))
                .collect(),
        ),
        Source::Completion(Value::Object(fields)) => Plain::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), convert(Source::Completion(value))))
                .collect(),
        ),
        Source::Prompt(Prompted::Object(fields)) => Plain::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), convert(Source::Prompt(value))))
                .collect(),
        ),
    }
}

mod tests;
