use std::error::Error;

use async_trait::async_trait;
use futures_util::future::join_all;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_actions::{ContentFilterSource, filter_single_text};

use super::error::ImageProcessingError;

pub const IMAGE_DESCRIPTION_PROMPT: &str = "Describe the image in detail.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDescriptionRequest {
    pub model: String,
    pub system_prompt: &'static str,
    pub image_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDescriptionReply {
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDescriptionResponse {
    pub choices: Vec<ImageDescriptionReply>,
}

// LiteLLM's llm_router.acompletion, which the server implements over the router
#[async_trait]
pub trait ImageDescriptionCompletion: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn describe(
        &self,
        request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error>;
}

#[derive(Debug)]
pub struct ImageScan<'scan, 'source> {
    pub images: &'scan [String],
    pub image_model: Option<&'scan str>,
    pub source: &'source ContentFilterSource<'source>,
}

pub async fn process_images<C>(
    scan: &ImageScan<'_, '_>,
    completion: Option<&C>,
    detections: &mut Vec<ContentFilterDetection>,
) -> Result<(), ImageProcessingError<C::Error>>
where
    C: ImageDescriptionCompletion + ?Sized,
{
    let (Some(image_model), Some(describer)) = (scan.image_model, completion) else {
        return Ok(());
    };
    if scan.images.is_empty() {
        return Ok(());
    }
    let descriptions = describe_images(scan, image_model, describer).await?;
    for description in &descriptions {
        match filter_single_text(scan.source, description, Some(detections)) {
            Ok(_filtered) => {}
            Err(ContentFilterActionError::Blocked { detail }) => {
                return Err(ImageProcessingError::Blocked {
                    detail: Box::new(detail),
                    text: description.clone(),
                });
            }
            Err(error) => return Err(ImageProcessingError::Filter(error)),
        }
    }
    Ok(())
}

async fn describe_images<C>(
    scan: &ImageScan<'_, '_>,
    image_model: &str,
    completion: &C,
) -> Result<Vec<String>, ImageProcessingError<C::Error>>
where
    C: ImageDescriptionCompletion + ?Sized,
{
    let requests: Vec<ImageDescriptionRequest> = scan
        .images
        .iter()
        .map(|image| ImageDescriptionRequest {
            model: image_model.to_owned(),
            system_prompt: IMAGE_DESCRIPTION_PROMPT,
            image_url: image.clone(),
        })
        .collect();
    let calls = requests.iter().map(|request| completion.describe(request));
    let responses = join_all(calls).await;

    let mut descriptions = Vec::with_capacity(responses.len());
    for answer in responses {
        let response = answer.map_err(|source| ImageProcessingError::Completion { source })?;
        let Some(choice) = response.choices.first() else {
            return Err(ImageProcessingError::NoChoices);
        };
        if let Some(description) = choice.content.as_deref().filter(|text| !text.is_empty()) {
            tracing::debug!(description, "Image description");
            descriptions.push(description.to_owned());
        } else {
            tracing::warn!("No image description found");
        }
    }

    tracing::debug!(
        count = descriptions.len(),
        "content filter applying guardrail to image descriptions"
    );
    Ok(descriptions)
}
