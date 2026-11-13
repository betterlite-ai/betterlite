use super::error::FileUploadValidationError;
use std::io::{BufRead, Seek, SeekFrom};
pub trait SeekableBody: BufRead + Seek {}
impl<T: BufRead + Seek> SeekableBody for T {}
pub enum FileBody<'a> {
    Bytes(&'a [u8]),
    Seekable(&'a mut dyn SeekableBody),
}
impl FileBody<'_> {
    pub fn size(&mut self) -> Result<u128, FileUploadValidationError> {
        match self {
            Self::Bytes(bytes) => Ok(u128::from(bytes.len())),
            Self::Seekable(reader) => {
                let position = reader.stream_position()?;
                let end = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(position))?;
                Ok(u128::from(end))
            }
        }
    }
    pub fn reset(&mut self) -> Result<(), FileUploadValidationError> {
        match self {
            Self::Bytes(_) => Ok(()),
            Self::Seekable(reader) => reader.rewind().map_err(FileUploadValidationError::from),
        }
    }
}
