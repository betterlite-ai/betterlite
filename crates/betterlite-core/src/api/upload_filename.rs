#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UploadFilenameError {
    #[error("Filename contains null byte")]
    NullByte,
    #[error("Empty or unsafe filename")]
    EmptyOrUnsafe,
}

pub fn safe_filename(filename: &str) -> Result<String, UploadFilenameError> {
    if filename.contains('\0') {
        return Err(UploadFilenameError::NullByte);
    }
    let name = filename.replace('\\', "/");
    let basename = name
        .rsplit('/')
        .next()
        .ok_or(UploadFilenameError::EmptyOrUnsafe)?;
    if basename.is_empty() || matches!(basename, "." | "..") {
        return Err(UploadFilenameError::EmptyOrUnsafe);
    }
    Ok(basename.to_owned())
}

mod tests;
