use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse, ImageUsage};

use super::call::{GenericParams, ImageGenerationCall};
use super::operations::BranchAnswer;

// mock_image_generation answers ImageResponse(data=[ImageObject(url=mock_response)]) for any non-None value
pub(super) fn mock_answer(
    generic: &GenericParams,
    call: &ImageGenerationCall,
) -> Option<BranchAnswer> {
    let mock = generic.mock_response.as_ref()?;
    Some(BranchAnswer {
        response: ImageResponse {
            created: call.now_seconds,
            data: Some(vec![ImageData {
                b64_json: None,
                revised_prompt: None,
                url: Some(mock.clone()),
            }]),
            usage: Some(ImageUsage::default()),
        },
        hidden_model: None,
    })
}
