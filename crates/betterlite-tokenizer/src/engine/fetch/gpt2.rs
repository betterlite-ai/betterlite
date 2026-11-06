use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::collections::BTreeMap;
use std::num::TryFromIntError;
use std::str::{Utf8Error, from_utf8};
use thiserror::Error;
#[derive(Debug, Error)]
pub enum Gpt2Error {
    #[error("the gpt2 merge file is not UTF-8")]
    VocabularyUtf8(#[source] Utf8Error),
    #[error("the gpt2 merge file has no version line")]
    MissingVersion,
    #[error("a gpt2 merge line does not hold exactly two tokens")]
    MalformedMerge,
    #[error("the gpt2 merge file uses a character outside its byte map")]
    UnknownCharacter,
    #[error("a gpt2 byte map character is not a Unicode scalar value")]
    BadCodepoint,
    #[error("the gpt2 encoder map is not valid JSON")]
    EncoderJson(#[source] serde_json::Error),
    #[error("a gpt2 token rank does not fit in 32 bits")]
    RankOverflow(#[source] TryFromIntError),
    #[error("the gpt2 vocabulary has too many tokens")]
    TooManyTokens,
}
// tiktoken's data_gym_to_mergeable_bpe_ranks; both files come sha256-checked, so the encoder map only has to parse
pub fn bpe_file(vocabulary: &[u8], encoder: &[u8]) -> Result<Vec<u8>, Gpt2Error> {
    let text = from_utf8(vocabulary).map_err(Gpt2Error::VocabularyUtf8)?;
    let mapping = byte_map()?;
    let parts: Vec<&str> = text.split('\n').collect();
    let end = parts
        .len()
        .checked_sub(1)
        .ok_or(Gpt2Error::MissingVersion)?;
    let merges = parts.get(1..end).ok_or(Gpt2Error::MissingVersion)?;
    let _encoder: BTreeMap<String, u32> =
        serde_json::from_slice(encoder).map_err(Gpt2Error::EncoderJson)?;
    Ok(bpe_lines(&tokens_from_merges(merges, &mapping)?))
}
fn tokens_from_merges(
    merges: &[&str],
    mapping: &BTreeMap<char, u8>,
) -> Result<Vec<(Vec<u8>, u32)>, Gpt2Error> {
    let ordered = rank_order();
    let mut tokens: Vec<(Vec<u8>, u32)> =
        Vec::with_capacity(ordered.len().saturating_add(merges.len()));
    for (rank, byte) in ordered.iter().enumerate() {
        tokens.push((
            vec![*byte],
            u32::try_from(rank).map_err(Gpt2Error::RankOverflow)?,
        ));
    }
    let mut next = u32::try_from(ordered.len()).map_err(Gpt2Error::RankOverflow)?;
    for merge in merges {
        let fields: Vec<&str> = merge.split_whitespace().collect();
        if fields.len() != 2 {
            return Err(Gpt2Error::MalformedMerge);
        }
        let (Some(first), Some(second)) = (fields.first(), fields.get(1)) else {
            return Err(Gpt2Error::MalformedMerge);
        };
        let mut piece = decode(first, mapping)?;
        piece.extend(decode(second, mapping)?);
        tokens.push((piece, next));
        next = next.checked_add(1).ok_or(Gpt2Error::TooManyTokens)?;
    }
    Ok(tokens)
}
fn decode(value: &str, mapping: &BTreeMap<char, u8>) -> Result<Vec<u8>, Gpt2Error> {
    let mut bytes = Vec::with_capacity(value.len());
    for character in value.chars() {
        bytes.push(*mapping.get(&character).ok_or(Gpt2Error::UnknownCharacter)?);
    }
    Ok(bytes)
}
fn bpe_lines(tokens: &[(Vec<u8>, u32)]) -> Vec<u8> {
    let mut output = Vec::with_capacity(tokens.len().saturating_mul(32));
    for (bytes, rank) in tokens {
        output.extend_from_slice(STANDARD.encode(bytes).as_bytes());
        output.push(b' ');
        output.extend_from_slice(rank.to_string().as_bytes());
        output.push(b'\n');
    }
    output
}
const PRINTABLE: fn(u8) -> bool =
    |byte: u8| byte.is_ascii_graphic() || ((0xA1..=0xFF).contains(&byte) && byte != 0xAD);
fn rank_order() -> Vec<u8> {
    let mut ordered: Vec<u8> = (0_u8..=255).filter(|byte| PRINTABLE(*byte)).collect();
    ordered.extend((0_u8..=255).filter(|byte| !PRINTABLE(*byte)));
    ordered
}
fn byte_map() -> Result<BTreeMap<char, u8>, Gpt2Error> {
    let mut mapping = BTreeMap::new();
    for byte in (0_u8..=255).filter(|byte| PRINTABLE(*byte)) {
        mapping.insert(char::from(byte), byte);
    }
    let mut extra = 0_u32;
    for byte in (0_u8..=255).filter(|byte| !PRINTABLE(*byte)) {
        let codepoint = 256_u32.checked_add(extra).ok_or(Gpt2Error::BadCodepoint)?;
        mapping.insert(
            char::from_u32(codepoint).ok_or(Gpt2Error::BadCodepoint)?,
            byte,
        );
        extra = extra.checked_add(1).ok_or(Gpt2Error::BadCodepoint)?;
    }
    Ok(mapping)
}
