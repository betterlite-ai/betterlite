use std::collections::BTreeMap;
use std::num::TryFromIntError;
use std::str::{Utf8Error, from_utf8};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use thiserror::Error;

const ENDOFTEXT: &str = "<|endoftext|>";
const STARTOFTEXT: &str = "<|startoftext|>";

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
    #[error("the gpt2 vocabulary does not match its encoder map")]
    Mismatch,
    #[error("a gpt2 token rank does not fit in 32 bits")]
    RankOverflow(#[source] TryFromIntError),
    #[error("the gpt2 vocabulary has too many tokens")]
    TooManyTokens,
}

// tiktoken's data_gym_to_mergeable_bpe_ranks, rewritten to the tiktoken file format the engine loads
pub fn bpe_file(vocabulary: &[u8], encoder: &[u8]) -> Result<Vec<u8>, Gpt2Error> {
    let text = from_utf8(vocabulary).map_err(Gpt2Error::VocabularyUtf8)?;
    let mapping = byte_map()?;
    let parts: Vec<&str> = text.split('\n').collect();
    let end = parts
        .len()
        .checked_sub(1)
        .ok_or(Gpt2Error::MissingVersion)?;
    let merges = parts.get(1..end).ok_or(Gpt2Error::MissingVersion)?;
    let ordered = rank_order();
    let capacity = ordered.len().saturating_add(merges.len());
    let mut tokens: Vec<(Vec<u8>, u32)> = Vec::with_capacity(capacity);
    for (rank, byte) in ordered.iter().enumerate() {
        let id = u32::try_from(rank).map_err(Gpt2Error::RankOverflow)?;
        tokens.push((vec![*byte], id));
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
        let mut piece = decode(first, &mapping)?;
        piece.extend(decode(second, &mapping)?);
        tokens.push((piece, next));
        next = next.checked_add(1).ok_or(Gpt2Error::TooManyTokens)?;
    }
    verify_encoder(encoder, &mapping, &tokens)?;
    Ok(bpe_lines(&tokens))
}

fn decode(value: &str, mapping: &BTreeMap<char, u8>) -> Result<Vec<u8>, Gpt2Error> {
    let mut bytes = Vec::with_capacity(value.len());
    for character in value.chars() {
        let byte = mapping.get(&character).ok_or(Gpt2Error::UnknownCharacter)?;
        bytes.push(*byte);
    }
    Ok(bytes)
}

fn verify_encoder(
    encoder: &[u8],
    mapping: &BTreeMap<char, u8>,
    tokens: &[(Vec<u8>, u32)],
) -> Result<(), Gpt2Error> {
    let entries: BTreeMap<String, u32> =
        serde_json::from_slice(encoder).map_err(Gpt2Error::EncoderJson)?;
    let mut expected: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
    for (token, rank) in entries {
        if token == ENDOFTEXT || token == STARTOFTEXT {
            continue;
        }
        expected.insert(decode(&token, mapping)?, rank);
    }
    let actual: BTreeMap<Vec<u8>, u32> = tokens
        .iter()
        .map(|(bytes, rank)| (bytes.clone(), *rank))
        .collect();
    if actual == expected {
        Ok(())
    } else {
        Err(Gpt2Error::Mismatch)
    }
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

fn rank_order() -> Vec<u8> {
    let printable =
        |byte: u8| byte.is_ascii_graphic() || ((0xA1..=0xFF).contains(&byte) && byte != 0xAD);
    let mut ordered: Vec<u8> = (0_u8..=255).filter(|byte| printable(*byte)).collect();
    ordered.extend((0_u8..=255).filter(|byte| !printable(*byte)));
    ordered
}

fn byte_map() -> Result<BTreeMap<char, u8>, Gpt2Error> {
    let ordered = rank_order();
    let mut mapping = BTreeMap::new();
    for byte in &ordered {
        mapping.insert(char::from(*byte), *byte);
    }
    let mut extra = 0_u32;
    for byte in 0_u8..=255 {
        if ordered.contains(&byte) {
            continue;
        }
        let codepoint = 256_u32.checked_add(extra).ok_or(Gpt2Error::BadCodepoint)?;
        let character = char::from_u32(codepoint).ok_or(Gpt2Error::BadCodepoint)?;
        mapping.insert(character, byte);
        extra = extra.checked_add(1).ok_or(Gpt2Error::BadCodepoint)?;
    }
    Ok(mapping)
}
