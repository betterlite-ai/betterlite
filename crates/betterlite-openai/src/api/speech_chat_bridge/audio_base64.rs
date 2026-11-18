use thiserror::Error;

const PAD: u8 = b'=';
const QUANTUM: usize = 4;
const BITS_PER_CHARACTER: u32 = 6;
const BITS_PER_BYTE: u32 = 8;
const BYTE_MASK: u32 = 0xFF;
const REMAINDER_MASK: usize = 3;
const UPPER_CASE_START: u8 = 26;
const DIGIT_START: u8 = 52;
const PLUS_VALUE: u8 = 62;
const SLASH_VALUE: u8 = 63;

// base64.b64decode over a str: stray bytes are skipped, a pad counts only after two data characters, and a pad run that fills the quantum ends the parse
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpeechAudioError {
    #[error("string argument should contain only ASCII characters")]
    NonAscii,
    #[error("Incorrect padding")]
    Padding,
    #[error(
        "Invalid base64-encoded string: number of data characters ({count}) cannot be 1 more than a multiple of 4"
    )]
    Length { count: usize },
}

// the response audio part is base64 text, and Python's decoder tolerates the whitespace and stray characters a provider emits
pub fn decode_chat_audio_base64(data: &str) -> Result<Vec<u8>, SpeechAudioError> {
    if !data.is_ascii() {
        return Err(SpeechAudioError::NonAscii);
    }
    let mut decoded = Vec::new();
    let mut left: u32 = 0;
    let mut leftbits: u32 = 0;
    let mut data_chars: usize = 0;
    let mut pads: usize = 0;
    for byte in data.bytes() {
        if byte == PAD {
            let quad_pos = data_chars & REMAINDER_MASK;
            if quad_pos < 2 {
                continue;
            }
            pads = pads.saturating_add(1);
            if quad_pos.saturating_add(pads) == QUANTUM {
                return Ok(decoded);
            }
            continue;
        }
        let Some(value) = value_of(byte) else {
            continue;
        };
        pads = 0;
        data_chars = data_chars.saturating_add(1);
        left = (left << BITS_PER_CHARACTER) | u32::from(value);
        leftbits = leftbits.saturating_add(BITS_PER_CHARACTER);
        if leftbits >= BITS_PER_BYTE {
            leftbits = leftbits.saturating_sub(BITS_PER_BYTE);
            let masked = (left >> leftbits) & BYTE_MASK;
            decoded.push(u8::try_from(masked).unwrap_or_default());
            left &= (1_u32 << leftbits).saturating_sub(1);
        }
    }
    match data_chars & REMAINDER_MASK {
        0 => Ok(decoded),
        1 => Err(SpeechAudioError::Length { count: data_chars }),
        _ => Err(SpeechAudioError::Padding),
    }
}

const fn value_of(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte.saturating_sub(b'A')),
        b'a'..=b'z' => Some(byte.saturating_sub(b'a').saturating_add(UPPER_CASE_START)),
        b'0'..=b'9' => Some(byte.saturating_sub(b'0').saturating_add(DIGIT_START)),
        b'+' => Some(PLUS_VALUE),
        b'/' => Some(SLASH_VALUE),
        _ => None,
    }
}
