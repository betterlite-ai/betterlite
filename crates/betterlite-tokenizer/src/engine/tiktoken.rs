use kitoken::convert::convert_tiktoken;
use kitoken::{Kitoken, Regex, SpecialToken, SpecialTokenKind, Split, SplitBehavior, SplitPattern};

use crate::engine::error::EngineError;

const CL100K_PATTERN: &str = "'(?i:[sdmt]|ll|ve|re)|[^\\r\\n\\p{L}\\p{N}]?+\\p{L}++|\\p{N}{1,3}+| ?[^\\s\\p{L}\\p{N}]++[\\r\\n]*+|\\s++$|\\s*[\\r\\n]|\\s+(?!\\S)|\\s";

const O200K_PATTERN: &str = "[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]*[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]+(?i:'s|'t|'re|'ve|'m|'ll|'d)?|[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]+[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]*(?i:'s|'t|'re|'ve|'m|'ll|'d)?|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+";

const R50K_PATTERN: &str =
    "'(?:[sdmt]|ll|ve|re)| ?\\p{L}++| ?\\p{N}++| ?[^\\s\\p{L}\\p{N}]++|\\s++$|\\s+(?!\\S)|\\s";

const NO_SPECIAL_KINDS: &[SpecialTokenKind] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Encoding {
    Cl100kBase,
    O200kBase,
    P50kBase,
    P50kEdit,
    O200kHarmony,
    R50kBase,
    Gpt2,
}

impl Encoding {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cl100kBase => "cl100k_base",
            Self::O200kBase => "o200k_base",
            Self::P50kBase => "p50k_base",
            Self::P50kEdit => "p50k_edit",
            Self::O200kHarmony => "o200k_harmony",
            Self::R50kBase => "r50k_base",
            Self::Gpt2 => "gpt2",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "cl100k_base" => Some(Self::Cl100kBase),
            "o200k_base" => Some(Self::O200kBase),
            "p50k_base" => Some(Self::P50kBase),
            "p50k_edit" => Some(Self::P50kEdit),
            "o200k_harmony" => Some(Self::O200kHarmony),
            "r50k_base" => Some(Self::R50kBase),
            "gpt2" => Some(Self::Gpt2),
            _ => None,
        }
    }

    const fn pattern(self) -> &'static str {
        match self {
            Self::Cl100kBase => CL100K_PATTERN,
            Self::O200kBase | Self::O200kHarmony => O200K_PATTERN,
            Self::P50kBase | Self::P50kEdit | Self::R50kBase | Self::Gpt2 => R50K_PATTERN,
        }
    }

    fn specials(self) -> Vec<SpecialToken> {
        match self {
            Self::Cl100kBase => vec![
                control(100_257, "<|endoftext|>"),
                control(100_258, "<|fim_prefix|>"),
                control(100_259, "<|fim_middle|>"),
                control(100_260, "<|fim_suffix|>"),
                control(100_276, "<|endofprompt|>"),
            ],
            Self::O200kBase => vec![
                control(199_999, "<|endoftext|>"),
                control(200_018, "<|endofprompt|>"),
            ],
            Self::P50kBase | Self::R50kBase | Self::Gpt2 => {
                vec![control(50_256, "<|endoftext|>")]
            }
            Self::P50kEdit => vec![
                control(50_256, "<|endoftext|>"),
                control(50_281, "<|fim_prefix|>"),
                control(50_282, "<|fim_middle|>"),
                control(50_283, "<|fim_suffix|>"),
            ],
            Self::O200kHarmony => harmony_specials(),
        }
    }
}

#[derive(Debug)]
pub struct TiktokenEngine {
    inner: Kitoken,
}

impl TiktokenEngine {
    pub fn from_bpe_file(encoding: Encoding, data: &[u8]) -> Result<Self, EngineError> {
        let mut definition = convert_tiktoken(data).map_err(EngineError::Vocabulary)?;
        definition.specials = encoding.specials();
        definition.config.split = vec![Split::Pattern {
            pattern: SplitPattern::Regex(
                Regex::new(encoding.pattern()).map_err(EngineError::Pattern)?,
            ),
            behavior: SplitBehavior::Isolate,
        }];
        let inner = Kitoken::from_definition(definition).map_err(EngineError::Initialize)?;
        Ok(Self { inner })
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u32>, EngineError> {
        self.inner
            .encode(text, NO_SPECIAL_KINDS)
            .map_err(EngineError::TiktokenEncode)
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String, EngineError> {
        let bytes = self
            .inner
            .decode(ids, true)
            .map_err(EngineError::TiktokenDecode)?;
        Ok(replacement_string(&bytes))
    }
}

fn control(id: u32, token: &str) -> SpecialToken {
    SpecialToken {
        id,
        bytes: token.as_bytes().to_vec(),
        kind: SpecialTokenKind::Control,
        ident: None,
        score: 0.0,
        extract: false,
    }
}

fn harmony_specials() -> Vec<SpecialToken> {
    let mut specials = vec![
        control(199_998, "<|startoftext|>"),
        control(199_999, "<|endoftext|>"),
        control(200_000, "<|reserved_200000|>"),
        control(200_001, "<|reserved_200001|>"),
        control(200_002, "<|return|>"),
        control(200_003, "<|constrain|>"),
        control(200_004, "<|reserved_200004|>"),
        control(200_005, "<|channel|>"),
        control(200_006, "<|start|>"),
        control(200_007, "<|end|>"),
        control(200_008, "<|message|>"),
        control(200_009, "<|reserved_200009|>"),
        control(200_010, "<|reserved_200010|>"),
        control(200_011, "<|reserved_200011|>"),
        control(200_012, "<|call|>"),
        control(200_018, "<|endofprompt|>"),
    ];
    specials.extend((200_013_u32..=201_087).map(|id| control(id, &format!("<|reserved_{id}|>"))));
    specials
}

// tiktoken decodes with errors="replace", so an invalid byte sequence becomes one replacement character
fn replacement_string(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{fffd}');
        }
    }
    text
}
