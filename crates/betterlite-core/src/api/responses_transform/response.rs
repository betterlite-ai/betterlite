use betterlite_interactions::types::json::{Json as InteractionJson, Object as InteractionObject};
use betterlite_interactions::types::response::InteractionsApiResponse;
use betterlite_openai::api::responses_objects::sdk::unions::{
    OutputMessageContentValue, SdkOutput,
};
use betterlite_openai::api::responses_objects::{
    ResponseApiUsage, ResponseOutputItem, ResponsesApiResponse,
};
use jiff::Timestamp;
use jiff::fmt::strtime;
use jiff::tz::TimeZone;
use serde_json::Number;

use super::error::ResponsesTransformError;

pub fn to_interactions_response(
    response: &ResponsesApiResponse,
    model: Option<&str>,
    zone: &TimeZone,
) -> Result<InteractionsApiResponse, ResponsesTransformError> {
    let created = local_created(response.created_at, zone)?;
    let (outputs, steps) = outputs_and_steps(&response.output);
    let reported_model = match model {
        Some(name) if !name.is_empty() => Some(name.to_owned()),
        Some(_) | None => response.model.clone(),
    };
    Ok(InteractionsApiResponse {
        id: Some(response.id.clone()),
        object: Some(String::from("interaction")),
        model: reported_model,
        agent: None,
        status: response.status.clone(),
        created: Some(created.clone()),
        updated: Some(created),
        outputs: Some(outputs),
        steps: Some(steps),
        usage: usage_object(response.usage.as_ref()),
        extra: InteractionObject::new(),
        hidden_params: InteractionObject::new(),
    })
}

fn outputs_and_steps(
    output: &[ResponseOutputItem],
) -> (Vec<InteractionObject>, Vec<InteractionObject>) {
    let mut outputs = Vec::new();
    let mut steps = Vec::new();
    for item in output {
        let mut contents = Vec::new();
        for text in item_texts(item) {
            outputs.push(text_record(&text));
            contents.push(text_record(&text));
        }
        if !contents.is_empty() {
            steps.push(model_output_step(contents));
        }
    }
    (outputs, steps)
}

fn item_texts(item: &ResponseOutputItem) -> Vec<String> {
    match item {
        ResponseOutputItem::Sdk(output) => sdk_texts(output),
        ResponseOutputItem::Message(message) => message
            .content
            .iter()
            .filter_map(|part| part.text.clone())
            .collect(),
        ResponseOutputItem::FunctionCall(_)
        | ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_)
        | ResponseOutputItem::Unknown(_) => Vec::new(),
    }
}

fn sdk_texts(output: &SdkOutput) -> Vec<String> {
    if let SdkOutput::OutputMessage(message) = output {
        return message.content.iter().filter_map(message_text).collect();
    }
    if let SdkOutput::Reasoning(reasoning) = output {
        return reasoning
            .content
            .iter()
            .flatten()
            .map(|content| content.text.clone())
            .collect();
    }
    Vec::new()
}

fn message_text(part: &OutputMessageContentValue) -> Option<String> {
    match part {
        OutputMessageContentValue::OutputText(text) => Some(text.text.clone()),
        OutputMessageContentValue::OutputRefusal(_) => None,
    }
}

fn text_record(text: &str) -> InteractionObject {
    let mut record = InteractionObject::new();
    record.insert(
        String::from("type"),
        InteractionJson::Text(String::from("text")),
    );
    record.insert(String::from("text"), InteractionJson::Text(text.to_owned()));
    record
}

fn model_output_step(contents: Vec<InteractionObject>) -> InteractionObject {
    let mut step = InteractionObject::new();
    step.insert(
        String::from("type"),
        InteractionJson::Text(String::from("model_output")),
    );
    step.insert(
        String::from("content"),
        InteractionJson::List(contents.into_iter().map(InteractionJson::Object).collect()),
    );
    step
}

fn usage_object(usage: Option<&ResponseApiUsage>) -> Option<InteractionObject> {
    let found = usage?;
    let mut object = InteractionObject::new();
    object.insert(
        String::from("total_input_tokens"),
        InteractionJson::Number(Number::from(found.input_tokens)),
    );
    object.insert(
        String::from("total_output_tokens"),
        InteractionJson::Number(Number::from(found.output_tokens)),
    );
    Some(object)
}

fn local_created(seconds: i64, zone: &TimeZone) -> Result<String, ResponsesTransformError> {
    let timestamp = Timestamp::from_second(seconds)
        .map_err(|source| ResponsesTransformError::CreatedAt { seconds, source })?;
    strtime::format("%Y-%m-%dT%H:%M:%S", &timestamp.to_zoned(zone.clone()))
        .map_err(|source| ResponsesTransformError::CreatedAt { seconds, source })
}
