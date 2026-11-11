use super::decode::ReplyRecord;
use super::error::ImageCallError;
use crate::image_models::response::{
    ImagesOnlyResponse, ModelResponse, optional_images, stability1_images, titan_images,
};
use crate::image_models::stability3;

// the B633 transform mutates the supplied record's data; its caller keeps the same response record
pub fn apply_reply(
    model_response: &mut ModelResponse,
    record: ReplyRecord,
) -> Result<(), ImageCallError> {
    match record {
        ReplyRecord::Titan(response) => model_response.data = titan_images(&response),
        ReplyRecord::Stability1(response) => model_response.data = stability1_images(&response),
        ReplyRecord::Stability3(response) => {
            stability3::check_finish_reasons(&response)?;
            model_response.data = optional_images(&ImagesOnlyResponse {
                images: response.images,
            });
        }
        ReplyRecord::Nova(response) => model_response.data = optional_images(&response),
    }
    Ok(())
}
