use regex::Regex;

use crate::adaptive_classification::request_type::RequestType;
use crate::posterior_math;
use crate::tier_artifact_predictor::error::PredictorError;

const CODE_KEYWORDS: [&str; 8] = [
    "def",
    "class",
    "function",
    "python",
    "javascript",
    "typescript",
    "sql",
    "code",
];
const MATHEMATICS_KEYWORDS: [&str; 7] = [
    "solve",
    "calculate",
    "equation",
    "probability",
    "theorem",
    "proof",
    "integral",
];
// Python's \w is str.isalnum() and the underscore, where the regex crate's \w also holds marks
const WORD_CHARACTER: &str = r"^[\p{L}\p{N}]$";
// (?:^|\s)[A-D][.)]\s, where Python's \s is the White_Space property and four separators below it
const MULTIPLE_CHOICE: &str =
    r"(?:^|[\p{White_Space}\x{1c}-\x{1f}])[A-D][.)][\p{White_Space}\x{1c}-\x{1f}]";

pub struct CohortMatcher {
    word: Regex,
    multiple_choice: Regex,
}

impl CohortMatcher {
    pub fn new() -> Result<Self, PredictorError> {
        let pattern = |expression: &str| {
            Regex::new(expression).map_err(|source| PredictorError::Pattern { source })
        };
        Ok(Self {
            word: pattern(WORD_CHARACTER)?,
            multiple_choice: pattern(MULTIPLE_CHOICE)?,
        })
    }

    #[must_use]
    pub fn similarity_cohort(&self, prompt: &str, request_type: RequestType) -> String {
        let characters = prompt.chars().collect::<Vec<char>>();
        let length = characters.len();
        let code = prompt.contains("```")
            || CODE_KEYWORDS
                .iter()
                .any(|keyword| self.contains_keyword(&characters, keyword));
        let mathematics = prompt.contains(['$', '='])
            || MATHEMATICS_KEYWORDS
                .iter()
                .any(|keyword| self.contains_keyword(&characters, keyword));
        let multiple_choice = self.multiple_choice.is_match(prompt);
        let non_ascii = characters
            .iter()
            .filter(|character| !character.is_ascii())
            .count();
        let intl = u8::from(posterior_math::mostly_non_ascii(non_ascii, length));
        format!(
            "{request_type}|{}|code={}|math={}|mc={}|intl={intl}",
            bucket(length),
            u8::from(code),
            u8::from(mathematics),
            u8::from(multiple_choice),
        )
    }

    // a pinned keyword matches where Python's \b sits on both sides and the letters fold alike
    fn contains_keyword(&self, characters: &[char], keyword: &str) -> bool {
        let needle = keyword.chars().collect::<Vec<char>>();
        let length = needle.len();
        characters
            .windows(length)
            .enumerate()
            .any(|(start, window)| {
                window
                    .iter()
                    .zip(needle.iter())
                    .all(|(subject, pattern)| case_folded_eq(*pattern, *subject))
                    && self.boundary(characters, start.checked_sub(1))
                    && self.boundary(characters, Some(start.saturating_add(length)))
            })
    }

    fn boundary(&self, characters: &[char], position: Option<usize>) -> bool {
        position.is_none_or(|index| {
            characters
                .get(index)
                .is_none_or(|character| !self.is_word(*character))
        })
    }

    fn is_word(&self, character: char) -> bool {
        if character == '_' {
            return true;
        }
        let mut buffer = [0_u8; 4];
        self.word.is_match(character.encode_utf8(&mut buffer))
    }
}

// re.IGNORECASE lowers both sides simply, and re._casefix adds the dotless i and the long s partners
fn case_folded_eq(pattern: char, subject: char) -> bool {
    let lowered = subject.to_lowercase().next().unwrap_or(subject);
    lowered == pattern.to_lowercase().next().unwrap_or(pattern)
        || matches!((pattern, lowered), ('i', '\u{131}') | ('s', '\u{17f}'))
}

const fn bucket(length: usize) -> &'static str {
    if length < 200 {
        "short"
    } else if length < 800 {
        "medium"
    } else if length < 2000 {
        "long"
    } else {
        "very_long"
    }
}
