use std::borrow::Cow;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_models::decisions::Json;

use crate::api::fal::edit::error::FalEditError;
use crate::api::image_input_options::mime::image_content_type_stream;

pub enum EditImage<'value> {
    Text(&'value str),
    Bytes(&'value [u8]),
    Path(&'value Path),
    Reader(&'value mut (dyn Read + Seek)),
    Tuple(&'value EditImage<'value>),
    Other(&'value Json),
}

pub fn read_image_bytes<'image>(
    image: &'image EditImage<'_>,
) -> Result<Cow<'image, [u8]>, FalEditError> {
    match image {
        EditImage::Bytes(bytes) => Ok(Cow::Borrowed(*bytes)),
        EditImage::Path(path) => {
            fs::read(path)
                .map(Cow::Owned)
                .map_err(|source| FalEditError::ReadPath {
                    path: path.to_path_buf(),
                    source,
                })
        }
        EditImage::Reader(reader) => read_reader(reader).map(Cow::Owned),
        EditImage::Tuple(inner) => read_image_bytes(*inner),
        EditImage::Text(_) => Err(FalEditError::UnsupportedImage { kind: "str" }),
        EditImage::Other(value) => Err(FalEditError::UnsupportedImage {
            kind: json_kind(value),
        }),
    }
}

fn read_reader(reader: &mut (dyn Read + Seek)) -> Result<Vec<u8>, FalEditError> {
    let position = reader
        .stream_position()
        .map_err(|source| FalEditError::Reader { source })?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|source| FalEditError::Reader { source })?;
    let mut data = Vec::new();
    reader
        .read_to_end(&mut data)
        .map_err(|source| FalEditError::Reader { source })?;
    reader
        .seek(SeekFrom::Start(position))
        .map_err(|source| FalEditError::Reader { source })?;
    Ok(data)
}

pub fn to_data_url(image: &EditImage<'_>) -> Result<String, FalEditError> {
    if let EditImage::Text(text) = image {
        return Ok((*text).to_owned());
    }
    let bytes = read_image_bytes(image)?;
    let mime = image_content_type_stream(&mut bytes.as_ref());
    Ok(format!(
        "data:{mime};base64,{}",
        BASE64.encode(bytes.as_ref())
    ))
}

fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_f64() => "float",
        Json::Number(_) => "int",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}
