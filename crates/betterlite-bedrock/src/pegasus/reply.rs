use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;

use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput};
use crate::image_models::params::{Field, Params};
use crate::image_models::sizing::parse_python_int;
use crate::pegasus::counter::TokenCounter;
use crate::pegasus::error::{BedrockFailure, PegasusError};
use crate::pegasus::response::{PegasusResponse, PegasusUsage};

const INPUT_TOKENS: &str = "x-amzn-bedrock-input-token-count";
const OUTPUT_TOKENS: &str = "x-amzn-bedrock-output-token-count";

pub struct ReplyContext<'request> {
    pub status: u16,
    pub headers: &'request [(String, String)],
    pub model: &'request str,
    pub messages: &'request [Message],
    pub now: f64,
}

pub async fn decode_pegasus_reply<C>(
    body: &[u8],
    context: &ReplyContext<'_>,
    response: &mut PegasusResponse,
    counter: &C,
) -> Result<(), PegasusError>
where
    C: TokenCounter + Sync,
{
    let raw = std::str::from_utf8(body).map_err(|_utf8| {
        bedrock_error(
            context,
            "Error parsing response: the body is not UTF-8".to_owned(),
        )
    })?;
    let record: Field = serde_json::from_str(raw).map_err(|source| {
        bedrock_error(
            context,
            format!("Error parsing response: {raw}, error: {source}"),
        )
    })?;
    tracing::debug!("twelvelabs pegasus response: {raw}");
    let Field::Mapping(fields) = &record else {
        return Err(PegasusError::ReplyNotObject);
    };
    let message = message_content(fields)?;
    let finish_reason = finish_reason(fields)?;
    assign_content(
        context,
        response,
        &record,
        message.as_deref(),
        finish_reason,
    )?;
    let prompt_tokens = prompt_tokens(context, counter).await?;
    let completion_tokens = completion_tokens(context, response, counter).await?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(PegasusError::TotalOverflow)?;
    response.created =
        i64::try_from(context.now).map_err(|_range| PegasusError::CreatedOutOfRange)?;
    response.model = context.model.to_owned();
    response.usage = PegasusUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    };
    Ok(())
}

fn message_content(fields: &Params) -> Result<Option<String>, PegasusError> {
    match fields.get("message") {
        None | Some(Field::Null) => Ok(None),
        Some(Field::Text(text)) => Ok((!text.is_empty()).then(|| text.clone())),
        Some(other) if other.truthy() => Err(PegasusError::MessageNotString),
        Some(_) => Ok(None),
    }
}

fn finish_reason(fields: &Params) -> Result<&'static str, PegasusError> {
    match fields.get("finishReason") {
        None => Ok(map_finish_reason("stop").reason.as_str()),
        Some(Field::Text(text)) => Ok(map_finish_reason(text).reason.as_str()),
        Some(Field::Null | Field::Bool(_) | Field::Int(_) | Field::Float(_)) => {
            Ok(map_finish_reason("stop").reason.as_str())
        }
        Some(Field::List(_) | Field::Mapping(_)) => Err(PegasusError::FinishReasonNotMappable),
    }
}

fn assign_content(
    context: &ReplyContext<'_>,
    response: &mut PegasusResponse,
    record: &Field,
    message: Option<&str>,
    finish_reason: &str,
) -> Result<(), PegasusError> {
    let Some(choice) = response.choices.first_mut() else {
        return Err(content_error(context, record));
    };
    let Some(content) = message else {
        return Err(content_error(context, record));
    };
    if choice.message.tool_calls.is_some() {
        return Err(content_error(context, record));
    }
    choice.message.content = Some(content.to_owned());
    choice.finish_reason = Some(finish_reason.to_owned());
    Ok(())
}

async fn prompt_tokens<C>(context: &ReplyContext<'_>, counter: &C) -> Result<i64, PegasusError>
where
    C: TokenCounter + Sync,
{
    let Some(value) = header(context.headers, INPUT_TOKENS).filter(|value| !value.is_empty())
    else {
        return counter
            .count_messages(context.messages)
            .await
            .map_err(|source| PegasusError::TokenCount { source });
    };
    parse_python_int(value).map_err(|source| PegasusError::TokenCountHeader {
        name: "x-amzn-bedrock-input-token-count",
        source,
    })
}

async fn completion_tokens<C>(
    context: &ReplyContext<'_>,
    response: &PegasusResponse,
    counter: &C,
) -> Result<i64, PegasusError>
where
    C: TokenCounter + Sync,
{
    let Some(value) = header(context.headers, OUTPUT_TOKENS).filter(|value| !value.is_empty())
    else {
        let text = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .ok_or(PegasusError::TokenCount {
                source: CountError::MissingInput,
            })?;
        return counter
            .count_text(text)
            .await
            .map_err(|source| PegasusError::TokenCount { source });
    };
    parse_python_int(value).map_err(|source| PegasusError::TokenCountHeader {
        name: "x-amzn-bedrock-output-token-count",
        source,
    })
}

fn header<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header_name, _value)| header_name.eq_ignore_ascii_case(name))
        .map(|(_name, value)| value.as_str())
}

fn content_error(context: &ReplyContext<'_>, record: &Field) -> PegasusError {
    bedrock_error(
        context,
        format!(
            "Error setting response content: Unable to set message content. Response: {record:?}"
        ),
    )
}

fn bedrock_error(context: &ReplyContext<'_>, message: String) -> PegasusError {
    let input = ErrorInput {
        status_code: context.status,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(context.headers.to_vec())),
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => PegasusError::Bedrock {
            status: context.status,
            error: BedrockFailure {
                cause: Box::new(error),
            },
        },
        Err(source) => PegasusError::Header(source),
    }
}
