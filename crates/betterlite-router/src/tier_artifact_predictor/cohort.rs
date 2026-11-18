use regex::{Regex, RegexBuilder};

use crate::adaptive_classification::request_type::RequestType;
use crate::posterior_math;
use crate::tier_artifact_predictor::error::PredictorError;

const CODE: &str = r"```|\b(def|class|function|python|javascript|typescript|sql|code)\b";
const MATHEMATICS: &str = r"\b(solve|calculate|equation|probability|theorem|proof|integral)\b|[$=]";
const MULTIPLE_CHOICE: &str = r"(?:^|\s)[A-D][.)]\s";

pub struct CohortMatcher {
    code: Regex,
    mathematics: Regex,
    multiple_choice: Regex,
}

impl CohortMatcher {
    pub fn new() -> Result<Self, PredictorError> {
        Ok(Self {
            code: insensitive(CODE)?,
            mathematics: insensitive(MATHEMATICS)?,
            multiple_choice: Regex::new(MULTIPLE_CHOICE)
                .map_err(|source| PredictorError::Pattern { source })?,
        })
    }

    // similarity_cohort: the request type, prompt length bucket and four pinned prompt flags
    #[must_use]
    pub fn similarity_cohort(&self, prompt: &str, request_type: RequestType) -> String {
        let length = prompt.chars().count();
        let length_bucket = bucket(length);
        let code = u8::from(self.code.is_match(prompt));
        let mathematics = u8::from(self.mathematics.is_match(prompt));
        let multiple_choice = u8::from(self.multiple_choice.is_match(prompt));
        let non_ascii = prompt
            .chars()
            .filter(|character| !character.is_ascii())
            .count();
        let intl = u8::from(posterior_math::mostly_non_ascii(non_ascii, length));
        format!(
            "{request_type}|{length_bucket}|code={code}|math={mathematics}|mc={multiple_choice}|intl={intl}"
        )
    }
}

fn insensitive(pattern: &str) -> Result<Regex, PredictorError> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|source| PredictorError::Pattern { source })
}

// the reference's length buckets over the prompt's code points
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
