use super::EditError;

const REMOTE_PREFIXES: [&str; 2] = ["http://", "https://"];

// LiteLLM's FileTypes: bytes, a file-like object, a str that is a URL or a file path, or a list of them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppliedFile {
    pub content: Vec<u8>,
    pub position: usize,
}

impl SuppliedFile {
    // tell, seek(0), read, seek(position) as a caller's open file handle sees it
    fn read_from_start(&mut self) -> Vec<u8> {
        let position = self.position;
        self.position = 0;
        let read = self.content.clone();
        self.position = position;
        read
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuppliedMedia {
    Bytes(Vec<u8>),
    Text(String),
    File(SuppliedFile),
    List(Vec<Self>),
}

#[derive(Debug)]
pub enum MaskInput<'media> {
    Absent,
    Null,
    Media(&'media mut SuppliedMedia),
}

pub(crate) fn read_media_bytes(
    media: &mut SuppliedMedia,
    field: &'static str,
    depth: i64,
    max_depth: i64,
) -> Result<Vec<u8>, EditError> {
    if depth > max_depth {
        return Err(EditError::MediaDepth { field });
    }
    match media {
        SuppliedMedia::Bytes(bytes) => Ok(bytes.clone()),
        SuppliedMedia::File(file) => Ok(file.read_from_start()),
        SuppliedMedia::List(items) => {
            let Some(first) = items.first_mut() else {
                return Err(EditError::UnsupportedMedia { field });
            };
            let nested = depth.saturating_add(1);
            read_media_bytes(first, field, nested, max_depth)
        }
        SuppliedMedia::Text(text)
            if REMOTE_PREFIXES
                .iter()
                .any(|prefix| text.starts_with(prefix)) =>
        {
            Err(EditError::UnsupportedMediaUrl { field })
        }
        SuppliedMedia::Text(_) => Err(EditError::UnsupportedMedia { field }),
    }
}

// LiteLLM's _fetch_remote_image takes image[0] when the value is a non-empty list, and nothing otherwise
pub(crate) fn is_remote_candidate(media: &SuppliedMedia) -> bool {
    let candidate = match media {
        SuppliedMedia::List(items) => match items.first() {
            Some(first) => first,
            None => return false,
        },
        SuppliedMedia::Bytes(_) | SuppliedMedia::File(_) | SuppliedMedia::Text(_) => media,
    };
    matches!(
        candidate,
        SuppliedMedia::Text(text)
            if REMOTE_PREFIXES
                .iter()
                .any(|prefix| text.starts_with(prefix))
    )
}
