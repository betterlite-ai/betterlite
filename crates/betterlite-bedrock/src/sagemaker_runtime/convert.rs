use betterlite_models::decisions::Json as Plain;
use betterlite_openai::sagemaker_completion::params::Options;
use betterlite_prompts::arguments::json::Json as Prompted;
use indexmap::IndexMap;

// JSON bytes preserve Number tokens that serde's untagged content visitor rejects as u128.
pub(crate) fn completion_body(
    body: &Options,
) -> Result<IndexMap<String, Plain>, serde_json::Error> {
    body.iter()
        .map(|(name, value)| {
            let plain: Plain = serde_json::from_slice(&serde_json::to_vec(value)?)?;
            Ok((name.clone(), plain))
        })
        .collect()
}

pub(crate) fn prompt_body(body: &IndexMap<String, Prompted>) -> IndexMap<String, Plain> {
    body.iter()
        .map(|(name, value)| (name.clone(), prompt_value(value)))
        .collect()
}

fn prompt_value(input: &Prompted) -> Plain {
    match input {
        Prompted::Null => Plain::Null,
        Prompted::Bool(flag) => Plain::Bool(*flag),
        Prompted::Number(number) => Plain::Number(number.clone()),
        Prompted::Text(text) => Plain::Text(text.clone()),
        Prompted::List(items) => Plain::List(items.iter().map(prompt_value).collect()),
        Prompted::Object(fields) => Plain::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), prompt_value(value)))
                .collect(),
        ),
    }
}

mod tests;
