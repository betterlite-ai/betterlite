use betterlite_prompts::media::base64_string::get_base64_str;

use crate::image_models::params::{Field, Params};
use crate::pegasus::error::PegasusError;

pub fn build_media_source(optional: &Params) -> Result<Option<Field>, PegasusError> {
    let direct = choose(optional.get("mediaSource"), optional.get("media_source"));
    if let Some(Field::Mapping(mapping)) = direct {
        return Ok(Some(Field::Mapping(mapping.clone())));
    }
    let base64_input = choose(optional.get("video_base64"), optional.get("base64_string"))
        .filter(|value| value.truthy());
    if let Some(value) = base64_input {
        let Field::Text(text) = value else {
            return Err(PegasusError::NotText {
                field: "video_base64",
            });
        };
        let mut source = Params::new();
        source.insert(
            "base64String".to_owned(),
            Field::Text(get_base64_str(text).to_owned()),
        );
        return Ok(Some(Field::Mapping(source)));
    }
    let s3_uri = choose(
        optional.get("video_s3_uri"),
        choose(optional.get("s3_uri"), optional.get("media_source_s3_uri")),
    )
    .filter(|value| value.truthy());
    let Some(uri) = s3_uri else {
        return Ok(None);
    };
    let mut location = Params::new();
    location.insert("uri".to_owned(), uri.clone());
    let bucket_owner = choose(
        optional.get("video_s3_bucket_owner"),
        choose(
            optional.get("s3_bucket_owner"),
            optional.get("media_source_bucket_owner"),
        ),
    )
    .filter(|value| value.truthy());
    if let Some(owner) = bucket_owner {
        location.insert("bucketOwner".to_owned(), owner.clone());
    }
    let mut source = Params::new();
    source.insert("s3Location".to_owned(), Field::Mapping(location));
    Ok(Some(Field::Mapping(source)))
}

fn choose<'field>(
    first: Option<&'field Field>,
    second: Option<&'field Field>,
) -> Option<&'field Field> {
    match first {
        Some(value) if value.truthy() => Some(value),
        _ => second,
    }
}
