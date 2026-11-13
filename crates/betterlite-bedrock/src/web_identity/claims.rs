use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, Engine as _};
use serde::Deserialize;
use unicode_general_category::{GeneralCategory, get_general_category};
const ENGINE: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);
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
    let claims: Claims = serde_json::from_slice(&decode_base64(payload)?).ok()?;
    if claims.aud.is_none() && claims.iss.is_none() {
        return None;
    }
    let aud = claims.aud.as_ref().map_or_else(
        || "None".to_owned(),
        |aud| match aud {
            Audience::Text(text) => claim_repr(text),
            Audience::List(items) => {
                let rendered: Vec<String> = items.iter().map(|item| claim_repr(item)).collect();
                format!("[{}]", rendered.join(", "))
            }
        },
    );
    let iss = claims
        .iss
        .as_ref()
        .map_or_else(|| "None".to_owned(), |value| claim_repr(value));
    Some(format!("aud={aud}, iss={iss}"))
}
// the aud/iss diagnostic prints Python's repr, so the quote and escapes follow str repr
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
    character == ' '
        || !matches!(
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
    let (mut symbols, pads) = base64_symbols(payload)?;
    let needed = match symbols.len().checked_rem(4)? {
        2 => 2_usize,
        3 => 1_usize,
        _ => 0_usize,
    };
    let added = payload.len().wrapping_neg().checked_rem(4).unwrap_or(0);
    if pads.saturating_add(added) < needed {
        return None;
    }
    while symbols.len().checked_rem(4) != Some(0) {
        symbols.push('=');
    }
    ENGINE.decode(symbols).ok()
}
fn base64_symbols(payload: &str) -> Option<(String, usize)> {
    let mut symbols = String::with_capacity(payload.len());
    let mut pads = 0_usize;
    for character in payload.chars() {
        match character {
            '-' | '_' | 'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => {
                symbols.push(match character {
                    '-' => '+',
                    '_' => '/',
                    other => other,
                });
                pads = 0;
            }
            '=' => pads = pads.saturating_add(1),
            '\u{0}'..='\u{7f}' => {}
            _ => return None,
        }
    }
    Some((symbols, pads))
}
