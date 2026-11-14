use super::error::PredictionError;

pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

// The REST transcoding of a server-streaming RPC sends one JSON array whose objects arrive across reads
#[derive(Debug, Default)]
pub struct PredictionFrames {
    level: usize,
    in_string: bool,
    escape_next: bool,
    current: Vec<u8>,
    started: bool,
    ended: bool,
}

impl PredictionFrames {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, PredictionError> {
        let mut ready = Vec::new();
        for &byte in chunk {
            self.step(byte, &mut ready)?;
        }
        Ok(ready)
    }

    pub fn finish(&self) -> Result<(), PredictionError> {
        match self.level {
            0 => Ok(()),
            _ => Err(PredictionError::StreamUnfinished),
        }
    }

    fn step(&mut self, byte: u8, ready: &mut Vec<Vec<u8>>) -> Result<(), PredictionError> {
        if !self.started {
            // the source's first-chunk check: the array must open on the first byte
            if byte != b'[' {
                return Err(PredictionError::StreamShape);
            }
            self.started = true;
            self.level = 1;
            return Ok(());
        }
        if self.ended {
            return if byte.is_ascii_whitespace() {
                Ok(())
            } else {
                Err(PredictionError::StreamShape)
            };
        }
        match byte {
            b'{' => {
                if self.level == 1 {
                    self.current.clear();
                }
                if !self.in_string {
                    self.level = self.level.saturating_add(1);
                }
                self.push_byte(byte)?;
            }
            b'}' => {
                self.push_byte(byte)?;
                if !self.in_string {
                    self.level = self.level.saturating_sub(1);
                }
                if !self.in_string && self.level == 1 {
                    ready.push(std::mem::take(&mut self.current));
                }
            }
            b'"' => {
                if !self.escape_next {
                    self.in_string = !self.in_string;
                }
                self.push_byte(byte)?;
            }
            b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
                if self.in_string {
                    self.push_byte(byte)?;
                }
            }
            b'[' => self.push_byte(byte)?,
            b']' => {
                if self.level == 1 {
                    self.level = 0;
                    self.ended = true;
                } else {
                    self.push_byte(byte)?;
                }
            }
            _ => self.push_byte(byte)?,
        }
        self.escape_next = if byte == b'\\' {
            !self.escape_next
        } else {
            false
        };
        Ok(())
    }

    fn push_byte(&mut self, byte: u8) -> Result<(), PredictionError> {
        if self.current.len() >= MAX_FRAME_BYTES {
            return Err(PredictionError::TooLarge {
                limit: MAX_FRAME_BYTES,
            });
        }
        self.current.push(byte);
        Ok(())
    }
}
