use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageParams, ImageResponse,
};
use betterlite_provider::api::image_contracts::edit::{
    BaseImageEditConfig, ImageEditReply, ImageEditRequest,
};
use betterlite_provider::api::image_contracts::error::ImageContractError;

use super::{EditFixture, Seen};

const RAISED: &str = "the fixture provider failed";

impl BaseImageEditConfig for EditFixture {
    type Error = ImageContractError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _image_edit_optional_params: &ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        ImageParams::new()
    }

    fn transform_image_edit_request(
        &self,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), Self::Error> {
        let seen = Seen {
            model: request.model.to_owned(),
            prompt: request.prompt.map(str::to_owned),
            image: request.image.cloned(),
            options: request.optional_params.clone(),
            headers: request.headers.clone(),
        };
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.raise_status.is_some() {
            return Err(ImageContractError::NotImplemented { message: RAISED });
        }
        let mut data = request.optional_params;
        data.insert(String::from("touched"), Json::Bool(true));
        let mut headers = request.headers;
        headers.insert(String::from("touched"), String::from("yes"));
        data.insert(
            String::from("header_touched"),
            Json::Bool(headers.contains_key("touched")),
        );
        let files: ImageFiles = vec![(
            String::from("image"),
            ImageFile {
                filename: Some(String::from("a.png")),
                content: vec![b'a'],
                content_type: Some(String::from("image/png")),
            },
        )];
        Ok((data, files))
    }

    fn transform_image_edit_response(
        &self,
        _reply: ImageEditReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        Ok(ImageResponse::default())
    }
}
