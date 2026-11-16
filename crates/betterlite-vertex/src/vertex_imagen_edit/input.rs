use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::PathBuf;

use betterlite_openai::openai_video_identifiers::multipart::ReadSeek;

use super::decode::decode_imagen_edit_input;
use super::error::ImagenEditError;

// _read_all_bytes: the file records LiteLLM's reader walks, projected from a request's FileTypes
pub enum ImagenFileInput<'file> {
    // a bytes value, used as it came
    Bytes(Vec<u8>),
    // a bytearray, whose in-place mutability the reader does not need
    ByteArray(Vec<u8>),
    // a bare str, which the reader refuses instead of decoding it as base64
    Text(String),
    // a filesystem path, which the reader refuses without opening it
    Path(PathBuf),
    // a list or tuple, read through its first non-None member
    Sequence(Vec<Option<ImagenFileInput<'file>>>),
    // a dict, read through data, bytes or content and then path
    Mapping(ImagenFields<'file>),
    // a BytesIO, read whole from position zero and left where it was found
    Seekable(&'file mut dyn ReadSeek),
    // a BufferedReader or BufferedRandom, whose tell may refuse to answer
    Buffered(&'file mut dyn ReadSeek),
    // any other object with read(), whose answer may be text
    Reader(&'file mut dyn ImagenFileRead),
    // every other record, reported by the class the projection saw
    Unsupported { kind: &'static str },
}

// the four keys of LiteLLM's dict branch, in the order its reader tries them
pub struct ImagenFields<'file> {
    pub data: Option<Box<ImagenFileInput<'file>>>,
    pub bytes: Option<Box<ImagenFileInput<'file>>>,
    pub content: Option<Box<ImagenFileInput<'file>>>,
    pub path: Option<Box<ImagenFileInput<'file>>>,
}

pub trait ImagenFileRead {
    fn read_all(&mut self) -> io::Result<ImagenReadResult>;
}

pub enum ImagenReadResult {
    Bytes(Vec<u8>),
    Text(String),
}

// _read_all_bytes: the first readable member of the record, or the reason there is none
pub fn read_image_bytes(
    input: &mut ImagenFileInput<'_>,
    depth: i64,
    max_depth: i64,
) -> Result<Vec<u8>, ImagenEditError> {
    if depth > max_depth {
        return Err(ImagenEditError::Depth { max_depth });
    }
    match input {
        ImagenFileInput::Bytes(content) | ImagenFileInput::ByteArray(content) => {
            Ok(content.clone())
        }
        ImagenFileInput::Text(_) => Err(ImagenEditError::PlainString),
        ImagenFileInput::Path(_) => Err(ImagenEditError::FilePath),
        ImagenFileInput::Sequence(items) => read_sequence(items, depth, max_depth),
        ImagenFileInput::Mapping(fields) => read_mapping(fields, depth, max_depth),
        ImagenFileInput::Seekable(stream) => read_seekable(&mut **stream),
        ImagenFileInput::Buffered(stream) => read_buffered(&mut **stream),
        ImagenFileInput::Reader(stream) => read_text_stream(&mut **stream),
        ImagenFileInput::Unsupported { kind } => {
            Err(ImagenEditError::UnsupportedImage { kind: *kind })
        }
    }
}

fn read_sequence(
    items: &mut [Option<ImagenFileInput<'_>>],
    depth: i64,
    max_depth: i64,
) -> Result<Vec<u8>, ImagenEditError> {
    let nested = depth.saturating_add(1);
    let Some(item) = items.iter_mut().flatten().next() else {
        return Err(ImagenEditError::UnsupportedImage {
            kind: "a sequence without a member",
        });
    };
    read_image_bytes(item, nested, max_depth)
}

fn read_mapping(
    fields: &mut ImagenFields<'_>,
    depth: i64,
    max_depth: i64,
) -> Result<Vec<u8>, ImagenEditError> {
    let nested = depth.saturating_add(1);
    for slot in [&mut fields.data, &mut fields.bytes, &mut fields.content] {
        let Some(value) = slot else {
            continue;
        };
        if let ImagenFileInput::Text(text) = value.as_mut() {
            match decode_imagen_edit_input(text) {
                Ok(content) => return Ok(content),
                // LiteLLM's except Exception: continue falls through to the next key
                Err(ImagenEditError::Base64(_)) => continue,
                Err(error) => return Err(error),
            }
        }
        return read_image_bytes(value.as_mut(), nested, max_depth);
    }
    let Some(path) = &mut fields.path else {
        return Err(ImagenEditError::UnsupportedImage {
            kind: "a mapping without an image key",
        });
    };
    read_image_bytes(path.as_mut(), nested, max_depth)
}

// LiteLLM rewinds a BytesIO, reads it whole and puts the cursor back where it was
fn read_seekable(stream: &mut dyn ReadSeek) -> Result<Vec<u8>, ImagenEditError> {
    let position = stream.stream_position().map_err(ImagenEditError::Stream)?;
    stream
        .seek(SeekFrom::Start(0))
        .map_err(ImagenEditError::Stream)?;
    let mut content = Vec::new();
    stream
        .read_to_end(&mut content)
        .map_err(ImagenEditError::Stream)?;
    stream
        .seek(SeekFrom::Start(position))
        .map_err(ImagenEditError::Stream)?;
    Ok(content)
}

// a BufferedReader whose tell refuses answers is read from where it stands and not rewound
fn read_buffered(stream: &mut dyn ReadSeek) -> Result<Vec<u8>, ImagenEditError> {
    let start = stream.stream_position().ok();
    if let Some(offset) = start {
        stream
            .seek(SeekFrom::Start(0))
            .map_err(ImagenEditError::Stream)?;
    }
    let mut content = Vec::new();
    stream
        .read_to_end(&mut content)
        .map_err(ImagenEditError::Stream)?;
    if let Some(offset) = start {
        stream
            .seek(SeekFrom::Start(offset))
            .map_err(ImagenEditError::Stream)?;
    }
    Ok(content)
}

// a generic file-like object: a text answer is UTF-8 encoded, as LiteLLM's encode call does
fn read_text_stream(stream: &mut dyn ImagenFileRead) -> Result<Vec<u8>, ImagenEditError> {
    let read = stream.read_all().map_err(ImagenEditError::Stream)?;
    Ok(match read {
        ImagenReadResult::Bytes(content) => content,
        ImagenReadResult::Text(text) => text.into_bytes(),
    })
}
