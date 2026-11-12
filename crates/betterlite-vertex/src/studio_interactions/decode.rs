use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::StudioInteractionError;
use super::records::{
    DeleteInteractionResult, HiddenParams, InteractionRecord, InteractionResponse, NativeRecord,
    StreamingRecord,
};

pub const RESPONSE_BODY_LIMIT: usize = 0x0200_0000;

#[derive(Debug, Clone, Copy)]
pub struct RawResponse<'raw> {
    pub status: u16,
    pub headers: &'raw IndexMap<String, String>,
    pub body: &'raw [u8],
}

fn gemini_error(response: &RawResponse<'_>) -> StudioInteractionError {
    StudioInteractionError::Gemini {
        status: response.status,
        body: str::from_utf8(response.body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", response.body.len()),
            str::to_owned,
        ),
        headers: response.headers.clone(),
    }
}

// Text that is not JSON is a Gemini error; JSON of the wrong shape is a record error.
pub fn decode_studio_interaction<R>(response: &RawResponse<'_>) -> Result<R, StudioInteractionError>
where
    R: NativeRecord,
{
    if response.body.len() > RESPONSE_BODY_LIMIT {
        return Err(StudioInteractionError::ResponseTooLarge {
            size: response.body.len(),
            limit: RESPONSE_BODY_LIMIT,
        });
    }
    let Ok(open) = serde_json::from_slice::<Json>(response.body) else {
        return Err(gemini_error(response));
    };
    let mut record: R = serde_json::from_slice(response.body)
        .map_err(|source| StudioInteractionError::Record { source })?;
    if let Json::Object(mut fields) = open {
        fields.retain(|name, _value| !R::KNOWN_FIELDS.contains(&name.as_str()));
        record.set_extension(fields);
    }
    Ok(record)
}

pub fn transform_response<P>(
    response: &RawResponse<'_>,
    project_headers: P,
) -> Result<InteractionResponse, StudioInteractionError>
where
    P: Fn(&IndexMap<String, String>) -> AdditionalHeaders,
{
    let record = decode_studio_interaction::<InteractionRecord>(response)?;
    Ok(InteractionResponse {
        record,
        hidden_params: HiddenParams {
            headers: response.headers.clone(),
            additional_headers: Some(project_headers(response.headers)),
        },
    })
}

pub fn transform_get_response(
    response: &RawResponse<'_>,
) -> Result<InteractionResponse, StudioInteractionError> {
    let record = decode_studio_interaction::<InteractionRecord>(response)?;
    Ok(InteractionResponse {
        record,
        hidden_params: HiddenParams {
            headers: response.headers.clone(),
            additional_headers: None,
        },
    })
}

// Any 2xx is success and its body is never read.
pub fn transform_delete_response(
    response: &RawResponse<'_>,
    interaction_id: &str,
) -> Result<DeleteInteractionResult, StudioInteractionError> {
    if (200..300).contains(&response.status) {
        return Ok(DeleteInteractionResult {
            success: true,
            id: interaction_id.to_owned(),
        });
    }
    Err(gemini_error(response))
}

pub fn transform_streaming_response(
    chunk: &Json,
) -> Result<StreamingRecord, StudioInteractionError> {
    let body =
        serde_json::to_vec(chunk).map_err(|source| StudioInteractionError::Record { source })?;
    let headers = IndexMap::new();
    decode_studio_interaction(&RawResponse {
        status: 200,
        headers: &headers,
        body: &body,
    })
}
