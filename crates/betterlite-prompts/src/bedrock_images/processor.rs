use betterlite_http::user_url::validate::Resolve;

use crate::bedrock_images::block::create_bedrock_block;
use crate::bedrock_images::error::BedrockImageError;
use crate::bedrock_images::formats::validate_format;
use crate::bedrock_images::parse::parse_mime_type;
use crate::bedrock_images::remote::RemoteImage;
use crate::bedrock_images::types::ContentBlock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base64Image {
    pub data: String,
    pub mime_type: String,
    pub format: String,
}

// _parse_base64_image: image_url.split(",") into exactly two parts, then the data: MIME descriptor
pub fn parse_base64_image(image_url: &str) -> Result<Base64Image, BedrockImageError> {
    let Some((image_metadata, data)) = image_url.split_once(',') else {
        return Err(BedrockImageError::DataUrlParts);
    };
    if data.contains(',') {
        return Err(BedrockImageError::DataUrlParts);
    }
    let Some(declared) = parse_mime_type(image_metadata) else {
        return Ok(Base64Image {
            data: data.to_owned(),
            mime_type: "image/jpeg".to_owned(),
            format: "jpeg".to_owned(),
        });
    };
    let mime_type = declared.split(';').next().unwrap_or_default();
    let format =
        mime_type
            .split('/')
            .nth(1)
            .ok_or_else(|| BedrockImageError::MissingMediaSubtype {
                mime_type: mime_type.to_owned(),
            })?;
    Ok(Base64Image {
        data: data.to_owned(),
        mime_type: mime_type.to_owned(),
        format: format.to_owned(),
    })
}

// process_image_sync and process_image_async: the substring dispatch, the format override, then validation
pub async fn process_image<R>(
    image_url: &str,
    format: Option<&str>,
    remote: &RemoteImage<R>,
) -> Result<ContentBlock, BedrockImageError>
where
    R: Resolve,
{
    let parsed = if image_url.contains("base64") {
        parse_base64_image(image_url)?
    } else if image_url.contains("http://") || image_url.contains("https://") {
        let (data, mime_type) = remote.image_details(image_url).await?;
        let image_format = mime_type
            .split('/')
            .nth(1)
            .ok_or_else(|| BedrockImageError::MissingMediaSubtype {
                mime_type: mime_type.clone(),
            })?
            .to_owned();
        Base64Image {
            data,
            mime_type,
            format: image_format,
        }
    } else {
        return Err(BedrockImageError::UnsupportedImageType);
    };
    let (mime_type, candidate_format) = match format {
        Some(overridden) => {
            let overridden_format = overridden.split('/').nth(1).ok_or_else(|| {
                BedrockImageError::MissingFormatSubtype {
                    format: overridden.to_owned(),
                }
            })?;
            (overridden.to_owned(), overridden_format.to_owned())
        }
        None => (parsed.mime_type, parsed.format),
    };
    let image_format = validate_format(&mime_type, &candidate_format)?;
    Ok(create_bedrock_block(
        &parsed.data,
        &mime_type,
        &image_format,
    ))
}
