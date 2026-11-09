use betterlite_prompts::media::base64_string::get_base64_str;

pub mod error;
pub mod fields;
pub mod params;
pub mod request;
mod tests;

use error::{Marengo3Error, bad_request};
use fields::InputType;
use params::{Marengo3Params, RawParams};
use request::{
    ImageInput, Marengo3Request, MediaSource, MultiInput, NamedMediaSource, RequestBody,
    S3Location, TextImageInput, TextInput, TimedMediaInput,
};

pub const MARENGO_3_MODEL_MARKER: &str = "marengo-embed-3-";
const S3_URI_PREFIX: &str = "s3://";

#[must_use]
pub fn is_marengo_3_model(model: Option<&str>) -> bool {
    let Some(name) = model else { return false };
    name.contains(MARENGO_3_MODEL_MARKER)
}

pub fn build_marengo_3_request(
    input: &str,
    inference_params: &RawParams,
    drop_params: bool,
) -> Result<Marengo3Request, Marengo3Error> {
    let params = Marengo3Params::validate(inference_params)?;
    let input_type = params.resolved_input_type();
    if !drop_params {
        reject_unsupported(&params, input_type)?;
    }
    let body = request_body(input, input_type, &params)?;
    Ok(Marengo3Request {
        inference_id: params.inference_id,
        body,
    })
}

fn reject_unsupported(params: &Marengo3Params, input_type: InputType) -> Result<(), Marengo3Error> {
    reject_given(
        &params.given_2_7_only,
        "Marengo 3.0 does not accept the Marengo 2.7 parameters",
    )?;
    if matches!(input_type, InputType::Video | InputType::Audio) {
        return Ok(());
    }
    reject_given(
        &params.timed.given(),
        &format!("Input type '{}' does not accept", input_type.name()),
    )
}

fn request_body(
    input: &str,
    input_type: InputType,
    params: &Marengo3Params,
) -> Result<RequestBody, Marengo3Error> {
    Ok(match input_type {
        InputType::Text => RequestBody::Text {
            text: TextInput {
                input_text: input.to_owned(),
            },
        },
        InputType::Image => RequestBody::Image {
            image: ImageInput {
                media_source: media_source(input, params.bucket_owner.as_deref())?,
            },
        },
        InputType::Video => RequestBody::Video {
            video: timed_media_input(input, params)?,
        },
        InputType::Audio => RequestBody::Audio {
            audio: timed_media_input(input, params)?,
        },
        InputType::TextImage => RequestBody::TextImage {
            text_image: text_image_input(input, params)?,
        },
        InputType::MultiInput => RequestBody::MultiInput {
            multi_input: multi_input(input, params)?,
        },
    })
}

fn reject_given(given: &[&str], reason: &str) -> Result<(), Marengo3Error> {
    if given.is_empty() {
        return Ok(());
    }
    Err(bad_request(format!(
        "{reason} {}; set drop_params to drop them",
        given.join(", ")
    )))
}

fn media_source(media: &str, bucket_owner: Option<&str>) -> Result<MediaSource, Marengo3Error> {
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

fn timed_media_input(
    media: &str,
    params: &Marengo3Params,
) -> Result<TimedMediaInput, Marengo3Error> {
    Ok(TimedMediaInput {
        media_source: media_source(media, params.bucket_owner.as_deref())?,
        options: params.timed.clone(),
    })
}

fn text_image_input(input: &str, params: &Marengo3Params) -> Result<TextImageInput, Marengo3Error> {
    let Some(media) = params.media_source.as_deref() else {
        return Err(bad_request(
            "Input type 'text_image' requires the 'media_source' parameter".to_owned(),
        ));
    };
    Ok(TextImageInput {
        input_text: input.to_owned(),
        media_source: media_source(media, params.bucket_owner.as_deref())?,
    })
}

fn multi_input(input: &str, params: &Marengo3Params) -> Result<MultiInput, Marengo3Error> {
    let Some(sources) = params
        .media_sources
        .as_ref()
        .filter(|named| !named.is_empty())
    else {
        return Err(bad_request(
            "Input type 'multi_input' requires a non-empty 'media_sources' mapping of name to media"
                .to_owned(),
        ));
    };
    let mut media_sources = Vec::with_capacity(sources.len());
    for (name, media) in sources {
        media_sources.push(NamedMediaSource {
            name: name.clone(),
            media_type: "image",
            source: media_source(media, params.bucket_owner.as_deref())?,
        });
    }
    Ok(MultiInput {
        input_text: (!input.is_empty()).then(|| input.to_owned()),
        media_sources,
    })
}
