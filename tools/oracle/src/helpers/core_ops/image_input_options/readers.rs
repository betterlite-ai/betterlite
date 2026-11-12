use std::io::{self, Cursor, Read, Seek, SeekFrom};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    None,
    Tell,
    Seek,
    Read,
    Restore,
    NonBytes,
}

pub(super) struct SeekableReader {
    pub(super) cursor: Cursor<Vec<u8>>,
    failure: Failure,
    start_seeks: usize,
}

impl SeekableReader {
    pub(super) fn new(data: Vec<u8>, position: usize, failure: Failure) -> Self {
        let mut cursor = Cursor::new(data);
        cursor.set_position(u64::try_from(position).unwrap_or(u64::MAX));
        Self {
            cursor,
            failure,
            start_seeks: 0,
        }
    }
}

impl Read for SeekableReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.failure {
            Failure::Read => Err(io::Error::other("read failed")),
            Failure::NonBytes => Ok(0),
            Failure::None | Failure::Tell | Failure::Seek | Failure::Restore => {
                self.cursor.read(buf)
            }
        }
    }
}

impl Seek for SeekableReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if self.failure == Failure::Tell && pos == SeekFrom::Current(0) {
            return Err(io::Error::other("tell failed"));
        }
        if matches!(pos, SeekFrom::Start(_)) {
            if self.failure == Failure::Seek && self.start_seeks == 0 {
                self.start_seeks = 1;
                return Err(io::Error::other("seek failed"));
            }
            if self.failure == Failure::Restore && self.start_seeks == 1 {
                self.start_seeks = 2;
                return Err(io::Error::other("restore failed"));
            }
            self.start_seeks = self.start_seeks.saturating_add(1);
        }
        self.cursor.seek(pos)
    }
}

pub(super) struct StreamReader {
    data: Vec<u8>,
    pub(super) position: usize,
    failure: Failure,
}

impl StreamReader {
    pub(super) const fn new(data: Vec<u8>, position: usize, failure: Failure) -> Self {
        Self {
            data,
            position,
            failure,
        }
    }
}

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.failure == Failure::Read {
            return Err(io::Error::other("read failed"));
        }
        let remaining = self.data.get(self.position..).unwrap_or_default();
        let count = remaining.len().min(buf.len());
        let target = buf
            .get_mut(..count)
            .ok_or_else(|| io::Error::other("buffer too small"))?;
        let source = remaining.get(..count).unwrap_or_default();
        target.copy_from_slice(source);
        self.position = self.position.saturating_add(count);
        Ok(count)
    }
}
