use super::error::OcrRequestError;

pub(super) struct Upload {
    pub(super) bytes: Vec<u8>,
    pub(super) position: usize,
}

// The rewind must happen before the size cap applies: middleware may have advanced the cursor.
pub(super) fn read_upload(upload: &mut Upload, cap: usize) -> Result<Vec<u8>, OcrRequestError> {
    upload.position = 0;
    if upload.bytes.is_empty() {
        return Err(OcrRequestError::EmptyFile);
    }
    if upload.bytes.len() > cap {
        return Err(OcrRequestError::FileTooLarge);
    }
    Ok(upload.bytes.clone())
}

mod tests;
