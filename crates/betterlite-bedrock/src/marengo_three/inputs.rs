use betterlite_prompts::media::base64_string::get_base64_str;

use super::error::{Marengo3Error, bad_request};
use super::params::Marengo3Params;
use super::request::{
    MediaSource, MultiInput, NamedMediaSource, S3Location, TextImageInput, TimedMediaInput,
};

const S3_URI_PREFIX: &str = "s3://";

pub(super) fn media_source(
    media: &str,
    bucket_owner: Option<&str>,
) -> Result<MediaSource, Marengo3Error> {
    if !media.starts_with(S3_URI_PREFIX) {
        return Ok(MediaSource::Inline {
            base64_string: get_base64_str(media).to_owned(),
        });
    }
    let Some(owner) = bucket_owner else {
        return Err(bad_request(
            "s3:// media requires the 'bucketOwner' parameter, the account id that owns the bucket"
                .to_owned(),
        ));
    };
    Ok(MediaSource::Remote {
        s3_location: S3Location {
            uri: media.to_owned(),
            bucket_owner: owner.to_owned(),
        },
    })
}

pub(super) fn timed_media_input(
    media: &str,
    params: &Marengo3Params,
) -> Result<TimedMediaInput, Marengo3Error> {
    media_source(media, params.bucket_owner.as_deref()).map(|media_source| TimedMediaInput {
        media_source,
        options: params.timed.clone(),
    })
}

pub(super) fn text_image_input(
    input: &str,
    params: &Marengo3Params,
) -> Result<TextImageInput, Marengo3Error> {
    let media = params.media_source.as_deref().ok_or_else(|| {
        bad_request("Input type 'text_image' requires the 'media_source' parameter".to_owned())
    })?;
    let source = media_source(media, params.bucket_owner.as_deref())?;
    Ok(TextImageInput {
        input_text: input.to_owned(),
        media_source: source,
    })
}

pub(super) fn multi_input(
    input: &str,
    params: &Marengo3Params,
) -> Result<MultiInput, Marengo3Error> {
    let sources = params
        .media_sources
        .as_ref()
        .filter(|named| !named.is_empty())
        .ok_or_else(|| {
            bad_request(
                "Input type 'multi_input' requires a non-empty 'media_sources' mapping of name to media"
                    .to_owned(),
            )
        })?;
    let owner = params.bucket_owner.as_deref();
    let media_sources = sources
        .iter()
        .map(|(name, media)| {
            Ok(NamedMediaSource {
                name: name.clone(),
                media_type: "image",
                source: media_source(media, owner)?,
            })
        })
        .collect::<Result<Vec<_>, Marengo3Error>>()?;
    Ok(MultiInput {
        input_text: (!input.is_empty()).then(|| input.to_owned()),
        media_sources,
    })
}
