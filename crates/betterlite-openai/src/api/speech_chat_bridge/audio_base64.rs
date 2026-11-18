use thiserror::Error;

const PAD: u8 = b'=';
const ALPHABET_BASE: u8 = 64;
const QUANTUM: usize = 4;
const BITS_PER_CHARACTER: u32 = 6;
const BITS_PER_BYTE: u32 = 8;
const BYTE_MASK: u32 = 0xFF;
const REMAINDER_MASK: usize = 3;
const UPPER_CASE_START: u8 = 26;
const DIGIT_START: u8 = 52;

// base64.b64decode over a str: bytes outside the alphabet are skipped, '=' counts only after the last data byte, and every partial quantum needs its pads
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
    let (decoded, data_chars, trailing_pads) = decode_quanta(data);
    check_padding(data_chars, trailing_pads)?;
    Ok(decoded)
}

// the decoded bytes, the data characters and the pads after the last one; an inner '=' is skipped like any other stray byte
fn decode_quanta(data: &str) -> (Vec<u8>, usize, usize) {
    let mut decoded = Vec::new();
    let mut left: u32 = 0;
    let mut leftbits: u32 = 0;
    let mut data_chars: usize = 0;
    let mut trailing_pads: usize = 0;
    for byte in data.bytes() {
        if byte == PAD {
            trailing_pads = trailing_pads.saturating_add(1);
            continue;
        }
        let Some(value) = value_of(byte) else {
            continue;
        };
        trailing_pads = 0;
        data_chars = data_chars.saturating_add(1);
        left = (left << BITS_PER_CHARACTER) | u32::from(value);
        leftbits = leftbits.saturating_add(BITS_PER_CHARACTER);
        if leftbits >= BITS_PER_BYTE {
            leftbits = leftbits.saturating_sub(BITS_PER_BYTE);
            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                reason = "the mask leaves only the low eight bits"
            )]
            let emitted = ((left >> leftbits) & BYTE_MASK) as u8;
            decoded.push(emitted);
            left &= (1 << leftbits).saturating_sub(1);
        }
    }
    (decoded, data_chars, trailing_pads)
}

// a lone data character can never complete a byte, and a partial quantum needs as many pads as it misses
fn check_padding(data_chars: usize, trailing_pads: usize) -> Result<(), SpeechAudioError> {
    let remainder = data_chars & REMAINDER_MASK;
    if remainder == 1 {
        return Err(SpeechAudioError::Length { count: data_chars });
    }
    let needed = match remainder {
        0 => 0,
        missing => QUANTUM.saturating_sub(missing),
    };
    if trailing_pads < needed {
        return Err(SpeechAudioError::Padding);
    }
    Ok(())
}

const fn value_of(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte.saturating_sub(b'A')),
        b'a'..=b'z' => Some(byte.saturating_sub(b'a').saturating_add(UPPER_CASE_START)),
        b'0'..=b'9' => Some(byte.saturating_sub(b'0').saturating_add(DIGIT_START)),
        b'+' => Some(62),
        b'/' => Some(ALPHABET_BASE.saturating_sub(1)),
        _ => None,
    }
}
