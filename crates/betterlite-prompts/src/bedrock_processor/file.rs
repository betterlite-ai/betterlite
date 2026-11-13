use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
use crate::bedrock_tool_results::invoke::CachePoints;

use super::conversation::Conversion;
use super::error::ProcessorError;
use super::media::image_block;

const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const FILE_ID: &str = "file_id";
const FORMAT: &str = "format";

pub async fn file_message_block<R, C>(
    fields: &Object,
    block: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Json, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let file = match fields.get(FILE) {
        None | Some(Json::Null) => {
            return Err(ProcessorError::FileObjectMissing { block });
        }
        Some(Json::Object(file)) => file,
        Some(_) => return Err(ProcessorError::FileNotObject { block }),
    };
    let file_data = match file.get(FILE_DATA) {
        None | Some(Json::Null) => None,
        Some(Json::Text(value)) => Some(value.as_str()),
        Some(_) => return Err(ProcessorError::FileDataNotText { block }),
    };
    let file_id = match file.get(FILE_ID) {
        None | Some(Json::Null) => None,
        Some(Json::Text(value)) => Some(value.as_str()),
        Some(_) => return Err(ProcessorError::FileIdNotText { block }),
    };
    let format = match file.get(FORMAT) {
        None | Some(Json::Null) => None,
        Some(Json::Text(value)) => Some(value.as_str()),
        Some(_) => return Err(ProcessorError::FileFormatNotText { block }),
    };
    let image_url = file_id
        .or(file_data)
        .ok_or(ProcessorError::FileWithoutMedia { block })?;
    image_block(image_url, format, block, conversion).await
}
