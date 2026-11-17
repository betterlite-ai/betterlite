use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_vertex::studio_image_edit::request::EditImages;
use betterlite_vertex::vertex_gemini_edit::request::GeminiEditFile;
use serde::Deserialize;
use serde_json::{Value, json};

// the call's image field: LiteLLM takes one record, or a list whose None members carry no image
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ImageField {
    One { record: ImageSpec },
    Many { records: Vec<Option<ImageSpec>> },
}

// the file record variants the edit reader accepts, plus one it rejects
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ImageSpec {
    Bytes { data_base64: String },
    Seekable { data_base64: String, position: u64 },
    Unsupported,
}

pub(super) enum Prepared {
    None,
    One(PreparedImage),
    Many(Vec<Option<PreparedImage>>),
}

pub(super) enum PreparedImage {
    Bytes(Vec<u8>),
    Seekable(Cursor<Vec<u8>>),
    Unsupported,
}

pub(super) fn prepared_field(field: Option<&ImageField>) -> Result<Prepared, Box<HelperError>> {
    match field {
        None => Ok(Prepared::None),
        Some(ImageField::One { record }) => Ok(Prepared::One(prepared_image(record)?)),
        Some(ImageField::Many { records }) => Ok(Prepared::Many(
            records
                .iter()
                .map(|record| record.as_ref().map(prepared_image).transpose())
                .collect::<Result<Vec<_>, _>>()?,
        )),
    }
}

fn prepared_image(spec: &ImageSpec) -> Result<PreparedImage, Box<HelperError>> {
    match spec {
        ImageSpec::Bytes { data_base64 } => Ok(PreparedImage::Bytes(
            STANDARD.decode(data_base64).or_config_error()?,
        )),
        ImageSpec::Seekable {
            data_base64,
            position,
        } => {
            let mut cursor = Cursor::new(STANDARD.decode(data_base64).or_config_error()?);
            cursor.set_position(*position);
            Ok(PreparedImage::Seekable(cursor))
        }
        ImageSpec::Unsupported => Ok(PreparedImage::Unsupported),
    }
}

pub(super) fn edit_images(prepared: &mut Prepared) -> Option<EditImages<'_>> {
    match prepared {
        Prepared::None => None,
        Prepared::One(record) => Some(EditImages::One(edit_file(record))),
        Prepared::Many(records) => Some(EditImages::Many(
            records
                .iter_mut()
                .map(|record| record.as_mut().map(edit_file))
                .collect(),
        )),
    }
}

fn edit_file(prepared: &mut PreparedImage) -> GeminiEditFile<'_> {
    match prepared {
        PreparedImage::Bytes(content) => GeminiEditFile::Bytes(content.as_slice()),
        PreparedImage::Seekable(cursor) => GeminiEditFile::Seekable(cursor),
        PreparedImage::Unsupported => GeminiEditFile::Unsupported,
    }
}

// the cursor each record reads from after the transform; only a seekable record has one
pub(super) fn positions_value(prepared: &Prepared) -> Value {
    let records: Vec<Option<&PreparedImage>> = match prepared {
        Prepared::None => Vec::new(),
        Prepared::One(record) => vec![Some(record)],
        Prepared::Many(records) => records.iter().map(Option::as_ref).collect(),
    };
    Value::Array(records.into_iter().map(record_position).collect())
}

fn record_position(record: Option<&PreparedImage>) -> Value {
    match record {
        Some(PreparedImage::Seekable(cursor)) => json!(cursor.position()),
        Some(PreparedImage::Bytes(_) | PreparedImage::Unsupported) | None => Value::Null,
    }
}
