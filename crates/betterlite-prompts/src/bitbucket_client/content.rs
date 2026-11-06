use std::string::FromUtf8Error;

const TEXT_PREFIX: &[u8] = b"text/";

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

// CPython 3.13's binascii.a2b_base64 outside strict mode skips bytes off the alphabet and needs the trailing padding to complete the last quad
fn decode_lenient(body: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(body.len());
    let mut quad_position = 0_u8;
    let mut trailing_pads = 0_usize;
    let mut carry = 0_u16;
    for byte in body {
        if *byte == b'=' {
            trailing_pads = trailing_pads.saturating_add(1);
            continue;
        }
        let Some(value) = sextet(*byte) else {
            continue;
        };
        trailing_pads = 0;
        match quad_position {
            0 => {
                carry = value;
                quad_position = 1;
            }
            1 => {
                decoded.push(u8::try_from(((carry << 2_u16) | (value >> 4_u16)) & 0xff_u16).ok()?);
                carry = value & 0xf_u16;
                quad_position = 2;
            }
            2 => {
                decoded.push(u8::try_from(((carry << 4_u16) | (value >> 2_u16)) & 0xff_u16).ok()?);
                carry = value & 0x3_u16;
                quad_position = 3;
            }
            _ => {
                decoded.push(u8::try_from(((carry << 6_u16) | value) & 0xff_u16).ok()?);
                quad_position = 0;
            }
        }
    }
    // a final quad of two sextets needs two pads and one of three needs one; a single sextet cannot exist
    let required_pads = match quad_position {
        0 => 0,
        2 => 2,
        3 => 1,
        _ => return None,
    };
    (trailing_pads >= required_pads).then_some(decoded)
}

// httpx would decode a body that is not UTF-8 with replacement characters; betterlite refuses it
pub fn decode_body(content_type: Option<&[u8]>, body: &[u8]) -> Result<String, FromUtf8Error> {
    let is_text = content_type.is_some_and(|value| value.starts_with(TEXT_PREFIX));
    if !is_text
        && let Some(decoded) = decode_lenient(body)
        && let Ok(text) = String::from_utf8(decoded)
    {
        return Ok(text);
    }
    String::from_utf8(body.to_vec())
}
