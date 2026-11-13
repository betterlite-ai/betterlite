use indexmap::IndexMap;
use serde_json::Value;

use crate::huggingface_chat::error::get_error_class;
use crate::huggingface_embedding::error::HuggingfaceEmbeddingError;
use crate::huggingface_embedding::task::{PipelineTagSource, resolve_task};
use crate::openai_base_config::{Headers, Object};

pub type Params = IndexMap<String, Value>;

const SENTENCE_TRANSFORMERS: &str = "sentence-transformers";
const SENTENCE_KEYS: (&str, &str) = ("source_sentence", "sentences");
const OPTION_PARAMS: [&str; 2] = ["use_cache", "wait_for_model"];
const PARAMETER_PARAMS: [&str; 7] = [
    "min_length",
    "max_length",
    "top_k",
    "top_p",
    "temperature",
    "repetition_penalty",
    "max_time",
];

pub fn transform_input(
    model: &str,
    input: &[String],
    optional_params: &Params,
    source: &dyn PipelineTagSource,
) -> Result<Object, HuggingfaceEmbeddingError> {
    if model.contains(SENTENCE_TRANSFORMERS) {
        let Some(first) = input.first() else {
            return Err(HuggingfaceEmbeddingError::Huggingface(get_error_class(
                "sentence transformers requires 2+ sentences",
                400,
                &Headers::new(),
            )));
        };
        let sentences = input.get(1..).unwrap_or_default();
        let data =
            Object::from_iter([("inputs".to_owned(), pair(SENTENCE_KEYS, first, sentences))]);
        return process_optional_params(data, optional_params);
    }
    let mut params = optional_params.clone();
    let task_type = params.shift_remove("input_type");
    let task = resolve_task(model, task_type.as_ref(), source)?;
    let data = input_for_tag(input, task.as_deref())?;
    process_optional_params(data, &params)
}

pub fn input_for_tag(
    input: &[String],
    pipeline_tag: Option<&str>,
) -> Result<Object, HuggingfaceEmbeddingError> {
    let inputs = match pipeline_tag {
        Some("sentence-similarity" | "similarity") => pair_inputs(
            input,
            SENTENCE_KEYS,
            "sentence-similarity requires 2+ sentences",
        )?,
        Some("rerank") => pair_inputs(input, ("query", "texts"), "reranker requires 2+ sentences")?,
        None | Some(_) => Value::Array(
            input
                .iter()
                .map(|text| Value::String(text.clone()))
                .collect(),
        ),
    };
    Ok(Object::from_iter([("inputs".to_owned(), inputs)]))
}

fn pair_inputs(
    input: &[String],
    keys: (&str, &str),
    message: &str,
) -> Result<Value, HuggingfaceEmbeddingError> {
    let Some((first, rest)) = input.split_first().filter(|(_, rest)| !rest.is_empty()) else {
        return Err(HuggingfaceEmbeddingError::Huggingface(get_error_class(
            message,
            400,
            &Headers::new(),
        )));
    };
    Ok(pair(keys, first, rest))
}

pub fn process_optional_params(
    mut data: Object,
    optional_params: &Params,
) -> Result<Object, HuggingfaceEmbeddingError> {
    for (name, value) in optional_params {
        if OPTION_PARAMS.contains(&name.as_str()) {
            merge_section(&mut data, "options", name, value)?;
        } else if PARAMETER_PARAMS.contains(&name.as_str()) {
            merge_section(&mut data, "parameters", name, value)?;
        } else {
            data.insert(name.clone(), value.clone());
        }
    }
    Ok(data)
}

fn pair(keys: (&str, &str), first: &str, rest: &[String]) -> Value {
    Value::Object(Object::from_iter([
        (keys.0.to_owned(), Value::String(first.to_owned())),
        (
            keys.1.to_owned(),
            Value::Array(
                rest.iter()
                    .map(|text| Value::String(text.clone()))
                    .collect(),
            ),
        ),
    ]))
}

fn merge_section(
    data: &mut Object,
    section: &'static str,
    name: &str,
    value: &Value,
) -> Result<(), HuggingfaceEmbeddingError> {
    let entry = data
        .entry(section.to_owned())
        .or_insert_with(|| Value::Object(Object::new()));
    let Value::Object(fields) = entry else {
        return Err(HuggingfaceEmbeddingError::SectionNotObject { section });
    };
    fields.insert(name.to_owned(), value.clone());
    Ok(())
}
