use std::io::{BufRead, Seek, SeekFrom};

use super::error::FileUploadValidationError;

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

    pub fn batch_size(&mut self) -> Result<u128, FileUploadValidationError> {
        match self {
            Self::Bytes(bytes) => Ok(u128::from(bytes.len())),
            Self::Seekable(reader) => {
                let end = reader.seek(SeekFrom::End(0))?;
                reader.seek(SeekFrom::Start(0))?;
                Ok(u128::from(end))
            }
        }
    }

    pub fn rewind(&mut self) -> Result<(), FileUploadValidationError> {
        match self {
            Self::Bytes(_) => Ok(()),
            Self::Seekable(reader) => {
                reader.rewind()?;
                Ok(())
            }
        }
    }
}
