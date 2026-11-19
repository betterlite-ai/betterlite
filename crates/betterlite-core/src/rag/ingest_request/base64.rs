use thiserror::Error;

const PADDING: u8 = b'=';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Base64DecodeError {
    #[error("string argument should contain only ASCII characters")]
    NonAscii,
    #[error("Incorrect padding")]
    IncorrectPadding,
    #[error(
        "Invalid base64-encoded string: number of data characters ({characters}) cannot be 1 more than a multiple of 4"
    )]
    OneModuloFour { characters: usize },
}

// CPython's base64.b64decode outside strict mode drops bytes off the alphabet, lets a trailing '=' run complete the last group, and rejects a lone final character by its count
pub(super) fn decode(text: &str) -> Result<Vec<u8>, Base64DecodeError> {
    if !text.is_ascii() {
        return Err(Base64DecodeError::NonAscii);
    }
    let (sextets, trailing_pads) = collect(text.as_bytes());
    let mut decoded = Vec::with_capacity(sextets.len());
    let mut quads = sextets.chunks_exact(4);
    for quad in &mut quads {
        let &[first, second, third, fourth] = quad else {
            continue;
        };
        decoded.push((first << 2) | (second >> 4));
        decoded.push(((second & 0x0f) << 4) | (third >> 2));
        decoded.push(((third & 0x03) << 6) | fourth);
    }
    match quads.remainder() {
        [] => Ok(decoded),
        [_] => Err(Base64DecodeError::OneModuloFour {
            characters: sextets.len(),
        }),
        [first, second] if trailing_pads >= 2 => {
            decoded.push((first << 2) | (second >> 4));
            Ok(decoded)
        }
        [_, _] => Err(Base64DecodeError::IncorrectPadding),
        [first, second, third] if trailing_pads >= 1 => {
            decoded.push((first << 2) | (second >> 4));
            decoded.push(((second & 0x0f) << 4) | (third >> 2));
            Ok(decoded)
        }
        [_, _, _] => Err(Base64DecodeError::IncorrectPadding),
        _ => Ok(decoded),
    }
}

fn collect(input: &[u8]) -> (Vec<u8>, usize) {
    let mut sextets = Vec::new();
    let mut trailing_pads = 0_usize;
    for byte in input {
        if *byte == PADDING {
            trailing_pads = trailing_pads.saturating_add(1);
            continue;
        }
        if let Some(value) = sextet(*byte) {
            sextets.push(value);
            trailing_pads = 0;
        }
    }
    (sextets, trailing_pads)
}

const fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => byte.checked_sub(b'A'),
        b'a'..=b'z' => match byte.checked_sub(b'a') {
            Some(value) => value.checked_add(26),
            None => None,
        },
        b'0'..=b'9' => match byte.checked_sub(b'0') {
            Some(value) => value.checked_add(52),
            None => None,
        },
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
