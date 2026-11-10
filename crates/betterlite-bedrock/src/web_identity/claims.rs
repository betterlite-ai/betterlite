use serde::Deserialize;
use unicode_general_category::{GeneralCategory, get_general_category};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Debug, Deserialize)]
struct Claims {
    #[serde(default)]
    aud: Option<Audience>,
    #[serde(default)]
    iss: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    Text(String),
    List(Vec<String>),
}

#[must_use]
pub fn decode_web_identity_claims(token: &str) -> Option<String> {
    let segments: Vec<&str> = token.split('.').collect();
    let [_, payload, _] = segments.as_slice() else {
        return None;
    };
    let decoded = decode_base64(payload)?;
    let claims: Claims = serde_json::from_slice(&decoded).ok()?;
    if claims.aud.is_none() && claims.iss.is_none() {
        return None;
    }
    let aud = claims
        .aud
        .as_ref()
        .map_or_else(|| "None".to_owned(), audience_repr);
    let iss = claims
        .iss
        .as_ref()
        .map_or_else(|| "None".to_owned(), |value| claim_repr(value));
    Some(format!("aud={aud}, iss={iss}"))
}

fn audience_repr(value: &Audience) -> String {
    match value {
        Audience::Text(text) => claim_repr(text),
        Audience::List(items) => {
            let rendered: Vec<String> = items.iter().map(|item| claim_repr(item)).collect();
            format!("[{}]", rendered.join(", "))
        }
    }
}

// Python's repr picks the quote from the content and escapes what str.isprintable rejects
fn claim_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(text.len().saturating_add(2));
    rendered.push(quote);
    for character in text.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\t' => rendered.push_str("\\t"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            ' ' => rendered.push(' '),
            _ if character == quote => {
                rendered.push('\\');
                rendered.push(quote);
            }
            _ if !printable(character) => rendered.push_str(&hex_escape(character)),
            _ => rendered.push(character),
        }
    }
    rendered.push(quote);
    rendered
}

fn printable(character: char) -> bool {
    !matches!(
        get_general_category(character),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::PrivateUse
            | GeneralCategory::Unassigned
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::SpaceSeparator
    )
}

fn hex_escape(character: char) -> String {
    let code = u32::from(character);
    match code {
        0..=0xff => format!("\\x{code:02x}"),
        0x100..=0xffff => format!("\\u{code:04x}"),
        _ => format!("\\U{code:08x}"),
    }
}

// Python's non-strict urlsafe_b64decode drops non-alphabet chars and starts padding from the raw length
pub(super) fn decode_base64(payload: &str) -> Option<Vec<u8>> {
    let symbols = base64_symbols(payload)?;
    let remainder = symbols.len().checked_rem(4).unwrap_or(0);
    let needed = match remainder {
        2 => 2_usize,
        3 => 1_usize,
        _ => 0_usize,
    };
    let added = match remainder {
        1 => return None,
        2 => 2_usize,
        3 => 1_usize,
        _ => 0_usize,
    };
    if padding_tail(payload).saturating_add(added) < needed {
        return None;
    }
    base64_bytes(&symbols)
}

fn base64_symbols(payload: &str) -> Option<Vec<u8>> {
    let mut symbols = Vec::new();
    for character in payload.chars() {
        let mapped = match character {
            '-' => '+',
            '_' => '/',
            '\u{0}'..='\u{7f}' => character,
            _ => return None,
        };
        let byte = u8::try_from(u32::from(mapped)).ok()?;
        if let Some(index) = ALPHABET.iter().position(|candidate| *candidate == byte) {
            symbols.push(u8::try_from(index).ok()?);
        }
    }
    Some(symbols)
}

fn padding_tail(payload: &str) -> usize {
    let mut pads = 0_usize;
    for character in payload.chars().rev() {
        if character == '=' {
            pads = pads.saturating_add(1);
            continue;
        }
        if character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '-' | '_') {
            break;
        }
    }
    pads
}

fn base64_bytes(symbols: &[u8]) -> Option<Vec<u8>> {
    let capacity = symbols
        .len()
        .checked_div(4)
        .unwrap_or(0)
        .saturating_mul(3)
        .saturating_add(3);
    let mut bytes = Vec::with_capacity(capacity);
    let mut bits = 0_u32;
    let mut count = 0_u32;
    for symbol in symbols {
        bits = bits.checked_shl(6).unwrap_or(0) | u32::from(*symbol);
        count = count.saturating_add(6);
        if count >= 8 {
            count = count.saturating_sub(8);
            let shifted = bits.checked_shr(count).unwrap_or(0) & 0xff_u32;
            bytes.push(u8::try_from(shifted).ok()?);
        }
    }
    Some(bytes)
}
