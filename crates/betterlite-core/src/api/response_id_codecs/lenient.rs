use std::string::FromUtf8Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum Base64Error {
    #[error("a base64 group ends after a single character")]
    SingleCharacter,
    #[error("the final base64 group lacks the padding it needs")]
    MissingPadding,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum TextDecodeError {
    #[error("{0}")]
    Base64(#[from] Base64Error),
    #[error("the decoded payload is not UTF-8: {0}")]
    Utf8(#[from] FromUtf8Error),
}

// CPython's base64.b64decode outside strict mode skips bytes off the alphabet, rejects a lone final character and needs the final group completed by padding
pub(super) fn decode_text(text: &str) -> Result<String, TextDecodeError> {
    match decode(text.as_bytes()) {
        Ok(decoded) => Ok(String::from_utf8(decoded)?),
        Err(error) => Err(TextDecodeError::from(error)),
    }
}

fn decode(input: &[u8]) -> Result<Vec<u8>, Base64Error> {
    let mut quad = Quad::default();
    for byte in input {
        if *byte == b'=' {
            quad.pads = quad.pads.saturating_add(1);
            continue;
        }
        let Some(value) = sextet(*byte) else {
            continue;
        };
        quad.push(value)?;
    }
    quad.finish()
}

fn sextet(byte: u8) -> Option<u16> {
    match byte {
        b'A'..=b'Z' => u16::from(byte).checked_sub(65),
        b'a'..=b'z' => u16::from(byte).checked_sub(71),
        b'0'..=b'9' => u16::from(byte).checked_add(4),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[derive(Default)]
struct Quad {
    position: u8,
    carry: u16,
    pads: usize,
    decoded: Vec<u8>,
}

impl Quad {
    fn push(&mut self, value: u16) -> Result<(), Base64Error> {
        self.pads = 0;
        match self.position {
            0 => {
                self.carry = value;
                self.position = 1;
            }
            1 => {
                self.push_byte(((self.carry << 2_u16).wrapping_add(value >> 4_u16)) & 0xff_u16)?;
                self.carry = value & 0xf_u16;
                self.position = 2;
            }
            2 => {
                self.push_byte(((self.carry << 4_u16).wrapping_add(value >> 2_u16)) & 0xff_u16)?;
                self.carry = value & 0x3_u16;
                self.position = 3;
            }
            _ => {
                self.push_byte((self.carry << 6_u16).wrapping_add(value) & 0xff_u16)?;
                self.position = 0;
            }
        }
        Ok(())
    }

    fn push_byte(&mut self, value: u16) -> Result<(), Base64Error> {
        let Ok(byte) = u8::try_from(value) else {
            return Err(Base64Error::SingleCharacter);
        };
        self.decoded.push(byte);
        Ok(())
    }

    fn finish(self) -> Result<Vec<u8>, Base64Error> {
        match (self.position, self.pads) {
            (0, _) => Ok(self.decoded),
            (1, _) => Err(Base64Error::SingleCharacter),
            (2, pads) if pads >= 2 => Ok(self.decoded),
            (3, pads) if pads >= 1 => Ok(self.decoded),
            _ => Err(Base64Error::MissingPadding),
        }
    }
}
