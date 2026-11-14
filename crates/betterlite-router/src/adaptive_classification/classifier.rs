use regex::{Regex, RegexBuilder};

use crate::adaptive_classification::error::AdaptiveClassificationError;
use crate::adaptive_classification::request_type::RequestType;

const MAX_PROMPT_CHARS: usize = 2000;

const RULES: [(&str, RequestType); 16] = [
    (
        r"\b(write|create|generate|implement|build)\s+(?:a |an |the |me )?(?:python|javascript|typescript|java|rust|go|c\+\+|sql|bash|shell)\b",
        RequestType::CodeGeneration,
    ),
    (
        r"\b(write|create|implement|build)\b(?:\s+\w+){0,4}?\s+(function|class|method|script|program|api|endpoint|microservice)\b",
        RequestType::CodeGeneration,
    ),
    (
        r"\b(explain|describe|understand|walk me through|what does)\b.*\b(code|function|method|class|algorithm|snippet)\b",
        RequestType::CodeUnderstanding,
    ),
    (
        r"\b(debug|fix|why (?:is|does|isn't)|what.s wrong|trace)\b.*\b(error|bug|exception|stacktrace|stack trace|traceback)\b",
        RequestType::CodeUnderstanding,
    ),
    (
        r"\b(review|critique)\s+(?:this |my |the )?(?:code|pr|pull request|diff|patch)\b",
        RequestType::CodeUnderstanding,
    ),
    (
        r"\b(design|architect|plan|architecture)\b.*\b(system|service|api|database|schema|module|microservice)\b",
        RequestType::TechnicalDesign,
    ),
    (
        r"\b(should i (?:use|choose|pick)|tradeoffs? between|compare)\b.*\b(library|framework|language|database|protocol|postgres|postgresql|mongodb|dynamodb|mysql|redis|kafka|sql|nosql)\b",
        RequestType::TechnicalDesign,
    ),
    (
        r"\bhow (?:should|do) i (?:design|structure|organize|model)\b",
        RequestType::TechnicalDesign,
    ),
    (
        r"\b(solve|compute|calculate|prove|derive)\b.*\b(equation|integral|derivative|theorem|proof|problem)\b",
        RequestType::AnalyticalReasoning,
    ),
    (
        r"\b(if .+ then|given .+ find|suppose|assume)\b",
        RequestType::AnalyticalReasoning,
    ),
    (
        r"\b(probability|statistics|combinatorics|optimization problem)\b",
        RequestType::AnalyticalReasoning,
    ),
    (
        r"\b(write|draft|compose|rewrite|edit|proofread|polish)\b.*\b(email|essay|blog|post|article|letter|memo|copy|paragraph|sentence)\b",
        RequestType::Writing,
    ),
    (
        r"\b(make (?:this|it)|help me)\s+(?:more |less )?(?:concise|formal|casual|professional|persuasive)\b",
        RequestType::Writing,
    ),
    (
        r"^\s*(who|what|when|where|which)\s+(?:is|was|were|are)\b",
        RequestType::FactualLookup,
    ),
    (
        r"^\s*(define|definition of|meaning of)\b",
        RequestType::FactualLookup,
    ),
    (
        r"^\s*how (?:do you spell|to spell|many .* are there|tall is)\b",
        RequestType::FactualLookup,
    ),
];

pub struct Classifier {
    rules: Vec<(Regex, RequestType)>,
}

impl Classifier {
    pub fn new() -> Result<Self, AdaptiveClassificationError> {
        let rules = RULES
            .iter()
            .enumerate()
            .map(|(index, (pattern, request_type))| {
                RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .map(|regex| (regex, *request_type))
                    .map_err(|source| AdaptiveClassificationError::ClassifierPattern {
                        index,
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { rules })
    }

    #[must_use]
    pub fn classify_prompt(&self, text: &str) -> RequestType {
        if text.trim().is_empty() {
            return RequestType::General;
        }
        let prompt = prompt_prefix(text);
        self.rules
            .iter()
            .find(|(pattern, _request_type)| pattern.is_match(prompt))
            .map_or(RequestType::General, |(_pattern, request_type)| {
                *request_type
            })
    }
}

fn prompt_prefix(text: &str) -> &str {
    text.char_indices()
        .nth(MAX_PROMPT_CHARS)
        .and_then(|(index, _character)| text.get(..index))
        .unwrap_or(text)
}
