use percent_encoding::percent_decode_str;

use crate::image_models::params::{Field, Params};

use super::error::NovaEmbeddingsError;

pub fn wrap_async_invoke_request(
    model_input: Params,
    model_id: &str,
    output_s3_uri: Option<&str>,
) -> Result<Params, NovaEmbeddingsError> {
    let decoded = percent_decode_str(model_id)
        .decode_utf8_lossy()
        .into_owned();
    let cleaned = if decoded.starts_with("async_invoke/") {
        decoded.replace("async_invoke/", "")
    } else {
        decoded
    };

    let Some(destination) = output_s3_uri else {
        return Err(NovaEmbeddingsError::OutputS3UriRequired);
    };
    if destination.trim().is_empty() {
        return Err(NovaEmbeddingsError::OutputS3UriRequired);
    }

    let mut s3_config = Params::new();
    s3_config.insert("s3Uri".to_owned(), Field::Text(destination.to_owned()));
    let mut output_data_config = Params::new();
    output_data_config.insert("s3OutputDataConfig".to_owned(), Field::Mapping(s3_config));

    let mut request = Params::new();
    request.insert("modelId".to_owned(), Field::Text(cleaned));
    request.insert("modelInput".to_owned(), Field::Mapping(model_input));
    request.insert(
        "outputDataConfig".to_owned(),
        Field::Mapping(output_data_config),
    );
    Ok(request)
}
