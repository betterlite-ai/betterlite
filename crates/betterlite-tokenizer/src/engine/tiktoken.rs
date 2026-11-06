use crate::engine::error::EngineError;
use kitoken::convert::convert_tiktoken;
use kitoken::{Kitoken, Regex, SpecialToken, SpecialTokenKind, Split, SplitBehavior, SplitPattern};
const CL100K_PATTERN: &str = "'(?i:[sdmt]|ll|ve|re)|[^\\r\\n\\p{L}\\p{N}]?+\\p{L}++|\\p{N}{1,3}+| ?[^\\s\\p{L}\\p{N}]++[\\r\\n]*+|\\s++$|\\s*[\\r\\n]|\\s+(?!\\S)|\\s";
const O200K_PATTERN: &str = "[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]*[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]+(?i:'s|'t|'re|'ve|'m|'ll|'d)?|[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]+[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]*(?i:'s|'t|'re|'ve|'m|'ll|'d)?|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+";
const R50K_PATTERN: &str =
    "'(?:[sdmt]|ll|ve|re)| ?\\p{L}++| ?\\p{N}++| ?[^\\s\\p{L}\\p{N}]++|\\s++$|\\s+(?!\\S)|\\s";
const NO_SPECIALS: &[SpecialTokenKind] = &[];
const CL100K: &[(u32, &str)] = &[
    (100_257, "<|endoftext|>"),
    (100_258, "<|fim_prefix|>"),
    (100_259, "<|fim_middle|>"),
    (100_260, "<|fim_suffix|>"),
    (100_276, "<|endofprompt|>"),
];
const O200K: &[(u32, &str)] = &[(199_999, "<|endoftext|>"), (200_018, "<|endofprompt|>")];
const R50K: &[(u32, &str)] = &[(50_256, "<|endoftext|>")];
const P50K_EDIT: &[(u32, &str)] = &[
    (50_256, "<|endoftext|>"),
    (50_281, "<|fim_prefix|>"),
    (50_282, "<|fim_middle|>"),
    (50_283, "<|fim_suffix|>"),
];
const HARMONY: &[(u32, &str)] = &[
    (199_998, "<|startoftext|>"),
    (199_999, "<|endoftext|>"),
    (200_002, "<|return|>"),
    (200_003, "<|constrain|>"),
    (200_005, "<|channel|>"),
    (200_006, "<|start|>"),
    (200_007, "<|end|>"),
    (200_008, "<|message|>"),
    (200_012, "<|call|>"),
    (200_018, "<|endofprompt|>"),
];
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
    const fn pattern(self) -> &'static str {
        match self {
            Self::Cl100kBase => CL100K_PATTERN,
            Self::O200kBase | Self::O200kHarmony => O200K_PATTERN,
            Self::P50kBase | Self::P50kEdit | Self::R50kBase | Self::Gpt2 => R50K_PATTERN,
        }
    }
    fn specials(self) -> Vec<SpecialToken> {
        let list = match self {
            Self::Cl100kBase => CL100K,
            Self::O200kBase => O200K,
            Self::P50kBase | Self::R50kBase | Self::Gpt2 => R50K,
            Self::P50kEdit => P50K_EDIT,
            Self::O200kHarmony => HARMONY,
        };
        let mut specials: Vec<SpecialToken> =
            list.iter().map(|(id, text)| control(*id, text)).collect();
        if self == Self::O200kHarmony {
            specials.extend(
                (200_000_u32..=201_087)
                    .filter(|id| !list.iter().any(|(known, _)| known == id))
                    .map(|id| control(id, &format!("<|reserved_{id}|>"))),
            );
        }
        specials
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
        match Kitoken::from_definition(definition) {
            Ok(inner) => Ok(Self { inner }),
            Err(failure) => Err(EngineError::Initialize(failure)),
        }
    }
    pub fn encode(&self, text: &str) -> Result<Vec<u32>, EngineError> {
        self.inner
            .encode(text, NO_SPECIALS)
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
// tiktoken decodes with errors="replace", so each invalid sequence becomes one replacement character
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
