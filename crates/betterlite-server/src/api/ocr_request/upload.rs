use super::error::OcrRequestError;

pub(super) fn read_upload(bytes: &[u8], cap: usize) -> Result<Vec<u8>, OcrRequestError> {
    if bytes.is_empty() {
        return Err(OcrRequestError::EmptyFile);
    }
    if bytes.len() > cap {
        return Err(OcrRequestError::FileTooLarge);
    }
    Ok(bytes.to_vec())
}

mod tests;
