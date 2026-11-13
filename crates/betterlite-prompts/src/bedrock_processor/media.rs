use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
use crate::bedrock_images::processor::process_image;
use crate::bedrock_tool_results::invoke::CachePoints;

use super::blocks::{IMAGE_URL, as_json};
use super::conversation::Conversion;
use super::error::ProcessorError;

const FORMAT: &str = "format";
const URL: &str = "url";

pub(crate) fn image_source(
    fields: &Object,
    block: usize,
) -> Result<(&str, Option<&str>), ProcessorError> {
    match fields.get(IMAGE_URL) {
        None => Err(ProcessorError::ImageUrlMissing { block }),
        Some(Json::Text(image_url)) => Ok((image_url.as_str(), None)),
        Some(Json::Object(image_object)) => {
            let url = match image_object.get(URL) {
                None => return Err(ProcessorError::ImageUrlWithoutAddress { block }),
                Some(Json::Text(url)) => url.as_str(),
                Some(_) => return Err(ProcessorError::ImageUrlNotText { block }),
            };
            let format = match image_object.get(FORMAT) {
                None | Some(Json::Null) => None,
                Some(Json::Text(format)) => Some(format.as_str()),
                Some(_) => return Err(ProcessorError::ImageFormatNotText { block }),
            };
            Ok((url, format))
        }
        Some(_) => Err(ProcessorError::ImageUrlNotTextOrObject { block }),
    }
}

pub(crate) async fn image_block<R, C>(
    image_url: &str,
    format: Option<&str>,
    block: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Json, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let content = process_image(image_url, format, conversion.remote)
        .await
        .map_err(|source| ProcessorError::Image { block, source })?;
    as_json(&content)
}
