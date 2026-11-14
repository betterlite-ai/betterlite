use betterlite_openai::chat::ToolCall;
use regex::RegexSet;

use crate::adaptive_classification::constants::{
    LOOP_REPEAT_THRESHOLD, MISALIGNMENT_JACCARD_THRESHOLD, STAGNATION_JACCARD_NEAR_DUP,
};
use crate::feedback_signals::error::SignalError;
use crate::feedback_signals::signal::SignalDelta;
use crate::feedback_signals::turn::{ToolResult, Turn};
use crate::signal_math::{jaccard, tokens};

// disengagement patterns come first, so their indexes sit below the satisfaction ones
const DETECTOR_PATTERNS: [&str; 6] = [
    r"(?i)\b(forget it|never mind|give up|talk to (?:a )?human|cancel)\b",
    r"(?i)\b(this (?:isn'?t|is not) working|stop|abort)\b",
    r"(?i)\bi'?ll do it (?:myself|manually)\b",
    r"(?i)\b(that worked|that did it|works now|fixed it|solved it|nice)\b",
    r"(?i)\b(thanks|thank you|thx|appreciated|appreciate it)\b",
    r"(?i)\b(perfect|great|excellent|exactly)\b",
];
const DISENGAGEMENT_PATTERN_COUNT: usize = 3;

const EXHAUSTION_STATUSES: [i64; 5] = [408, 413, 429, 503, 504];
const EXHAUSTION_KEYWORDS: [&str; 6] = [
    "context length",
    "context window",
    "token limit",
    "rate limit",
    "too many requests",
    "timeout",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorThresholds {
    misalignment: f64,
    stagnation: f64,
}

impl DetectorThresholds {
    pub const PINNED: Self = Self {
        misalignment: MISALIGNMENT_JACCARD_THRESHOLD,
        stagnation: STAGNATION_JACCARD_NEAR_DUP,
    };

    #[must_use]
    pub const fn checked(misalignment: f64, stagnation: f64) -> Result<Self, SignalError> {
        if !misalignment.is_finite() {
            return Err(SignalError::NonFinite {
                field: "misalignment",
                value: misalignment,
            });
        }
        if !stagnation.is_finite() {
            return Err(SignalError::NonFinite {
                field: "stagnation",
                value: stagnation,
            });
        }
        Ok(Self {
            misalignment,
            stagnation,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Detector {
    thresholds: DetectorThresholds,
    patterns: RegexSet,
}

impl Detector {
    pub fn new(thresholds: DetectorThresholds) -> Result<Self, SignalError> {
        Ok(Self {
            thresholds,
            patterns: RegexSet::new(DETECTOR_PATTERNS)
                .map_err(|source| SignalError::Pattern { source })?,
        })
    }
}

#[must_use]
pub fn detect_user_feedback(
    detector: &Detector,
    previous_user_content: Option<&str>,
    turn: &Turn,
    allow_satisfaction: bool,
) -> SignalDelta {
    SignalDelta {
        misalignment: i64::from(detect_misalignment(
            detector,
            previous_user_content,
            turn.user_content.as_deref(),
        )),
        stagnation: 0,
        disengagement: i64::from(detect_disengagement(detector, turn.user_content.as_deref())),
        satisfaction: i64::from(
            allow_satisfaction && detect_satisfaction(detector, turn.user_content.as_deref()),
        ),
        failure: i64::from(detect_failure(&turn.tool_results)),
        r#loop: 0,
        exhaustion: 0,
    }
}

#[must_use]
pub fn detect_response_signals(
    detector: &Detector,
    previous_assistant_content: Option<&str>,
    tool_call_history: &[String],
    turn: &Turn,
) -> SignalDelta {
    SignalDelta {
        misalignment: 0,
        stagnation: i64::from(detect_stagnation(
            detector,
            previous_assistant_content,
            turn.assistant_content.as_deref(),
        )),
        disengagement: 0,
        satisfaction: 0,
        failure: 0,
        r#loop: i64::from(detect_loop(tool_call_history, &turn.tool_calls)),
        exhaustion: i64::from(detect_exhaustion(turn.response_status, &turn.tool_results)),
    }
}

fn detect_misalignment(detector: &Detector, previous: Option<&str>, current: Option<&str>) -> bool {
    let previous_tokens = tokens(previous);
    let current_tokens = tokens(current);
    let similarity = jaccard(&previous_tokens, &current_tokens);
    0.0 < similarity && similarity < detector.thresholds.misalignment
}

fn detect_stagnation(detector: &Detector, previous: Option<&str>, current: Option<&str>) -> bool {
    let previous_tokens = tokens(previous);
    let current_tokens = tokens(current);
    let similarity = jaccard(&previous_tokens, &current_tokens);
    similarity >= detector.thresholds.stagnation
}

fn detect_disengagement(detector: &Detector, text: Option<&str>) -> bool {
    let Some(content) = text else {
        return false;
    };
    detector
        .patterns
        .matches(content)
        .iter()
        .any(|index| index < DISENGAGEMENT_PATTERN_COUNT)
}

fn detect_satisfaction(detector: &Detector, text: Option<&str>) -> bool {
    let Some(content) = text else {
        return false;
    };
    detector
        .patterns
        .matches(content)
        .iter()
        .any(|index| index >= DISENGAGEMENT_PATTERN_COUNT)
}

fn detect_failure(tool_results: &[ToolResult]) -> bool {
    for result in tool_results {
        if result.is_error {
            return true;
        }
    }
    false
}

fn detect_loop(history: &[String], calls: &[ToolCall]) -> bool {
    let needed = usize::try_from(LOOP_REPEAT_THRESHOLD)
        .unwrap_or(usize::MAX)
        .saturating_sub(1);
    for call in calls {
        let signature = format!("{}({})", call.function.name, call.function.arguments);
        if history.iter().filter(|known| **known == signature).count() >= needed {
            return true;
        }
    }
    false
}

fn detect_exhaustion(status: Option<i64>, tool_results: &[ToolResult]) -> bool {
    if status.is_some_and(|status| EXHAUSTION_STATUSES.contains(&status)) {
        return true;
    }
    for result in tool_results {
        let content = result.content.as_deref().unwrap_or_default().to_lowercase();
        if EXHAUSTION_KEYWORDS
            .iter()
            .any(|keyword| content.contains(keyword))
        {
            return true;
        }
    }
    false
}
