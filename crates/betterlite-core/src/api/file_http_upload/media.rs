use betterlite_openai::api::file_type_records::ByteStream;

// _MEDIA_UPLOAD_BLOCK_SIZE: one stream piece per JSONL row is regrouped into blocks this size, and
// never more than one block is buffered
pub const MEDIA_UPLOAD_BLOCK_SIZE: usize = 4 * 1024 * 1024;

// _iter_in_blocks: a block leaves as soon as the buffer holds one, and the last partial block leaves
// only when the stream is over, so an empty stream yields nothing at all
pub struct BlockIter {
    pieces: ByteStream,
    buffer: Vec<u8>,
}

impl BlockIter {
    #[must_use]
    pub fn new(pieces: ByteStream) -> Self {
        Self {
            pieces,
            buffer: Vec::new(),
        }
    }
}

impl Iterator for BlockIter {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            if self.buffer.len() >= MEDIA_UPLOAD_BLOCK_SIZE {
                let rest = self.buffer.split_off(MEDIA_UPLOAD_BLOCK_SIZE);
                return Some(std::mem::replace(&mut self.buffer, rest));
            }
            match self.pieces.next() {
                Some(piece) => self.buffer.extend_from_slice(&piece),
                None if self.buffer.is_empty() => return None,
                None => return Some(std::mem::take(&mut self.buffer)),
            }
        }
    }
}
