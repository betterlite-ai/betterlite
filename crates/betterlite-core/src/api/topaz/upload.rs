use betterlite_openai::api::image_contracts::records::{ImageFile, ImageHeaders, ImageParams};

pub const DEFAULT_FILENAME: &str = "image.png";
pub const DEFAULT_CONTENT_TYPE: &str = "image/png";

// prepare_file_tuple's FileTypes argument: the shapes it reads members from, and the ones it leaves alone
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopazFile<'file> {
    // bytes and BytesIO
    Bytes(&'file [u8]),
    // (filename, content)
    Named {
        filename: Option<&'file str>,
        content: &'file [u8],
    },
    // (filename, content, content_type)
    Typed {
        filename: Option<&'file str>,
        content: &'file [u8],
        content_type: Option<&'file str>,
    },
    // (filename, content, content_type, headers)
    Headered {
        filename: Option<&'file str>,
        content: &'file [u8],
        content_type: Option<&'file str>,
        headers: &'file ImageHeaders,
    },
    // any other FileTypes member, such as a path, keeps the defaults and a null content
    Unsupported,
}

// the source's tuple with its defaults resolved; only an unsupported shape keeps content null
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedFile {
    pub filename: String,
    pub content: Option<Vec<u8>>,
    pub content_type: String,
    pub headers: ImageHeaders,
}

impl Default for PreparedFile {
    fn default() -> Self {
        Self {
            filename: String::from(DEFAULT_FILENAME),
            content: None,
            content_type: String::from(DEFAULT_CONTENT_TYPE),
            headers: ImageHeaders::new(),
        }
    }
}

// TopazImageVariationConfig.prepare_file_tuple: a falsey filename or MIME falls back, and headers are taken as given
#[must_use]
pub fn prepare_file_tuple(file: TopazFile<'_>) -> PreparedFile {
    let mut prepared = PreparedFile::default();
    match file {
        TopazFile::Bytes(content) => prepared.content = Some(content.to_vec()),
        TopazFile::Named { filename, content } => {
            prepared.filename = text_or_default(filename, DEFAULT_FILENAME);
            prepared.content = Some(content.to_vec());
        }
        TopazFile::Typed {
            filename,
            content,
            content_type,
        } => {
            prepared.filename = text_or_default(filename, DEFAULT_FILENAME);
            prepared.content = Some(content.to_vec());
            prepared.content_type = text_or_default(content_type, DEFAULT_CONTENT_TYPE);
        }
        TopazFile::Headered {
            filename,
            content,
            content_type,
            headers,
        } => {
            prepared.filename = text_or_default(filename, DEFAULT_FILENAME);
            prepared.content = Some(content.to_vec());
            prepared.content_type = text_or_default(content_type, DEFAULT_CONTENT_TYPE);
            prepared.headers = headers.clone();
        }
        TopazFile::Unsupported => {}
    }
    prepared
}

// x or default: an absent or empty member keeps the source's default
fn text_or_default(value: Option<&str>, fallback: &str) -> String {
    match value {
        Some(text) if !text.is_empty() => text.to_owned(),
        Some(_) | None => fallback.to_owned(),
    }
}

// the multipart field the one image rides under
pub const IMAGE_FIELD: &str = "image";

// transform_request_image_variation's fields: the one image file part under its wire name and the mapped
// data, never the model
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopazFormFields {
    pub files: Vec<(String, PreparedFile)>,
    pub data: ImageParams,
}

#[must_use]
pub fn transform_request(file: TopazFile<'_>, data: ImageParams) -> TopazFormFields {
    TopazFormFields {
        files: vec![(IMAGE_FIELD.to_owned(), prepare_file_tuple(file))],
        data,
    }
}

// a variation request's typed upload is the source's (filename, content, content_type) tuple
impl<'file> From<&'file ImageFile> for TopazFile<'file> {
    fn from(image: &'file ImageFile) -> Self {
        Self::Typed {
            filename: image.filename.as_deref(),
            content: image.content.as_slice(),
            content_type: image.content_type.as_deref(),
        }
    }
}
