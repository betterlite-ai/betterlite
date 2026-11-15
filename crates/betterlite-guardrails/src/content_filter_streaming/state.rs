use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_matching::CategoryMatch;

mod tests;

// The check a detection came from, in the order _filter_single_text runs the checks
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DetectionCheck {
    Conditional,
    Phrase,
    CategoryKeyword,
    Pattern,
    BlockedWord,
}

// A detection with the position its check found it at, so a trimmed stream rebuilds scan order
#[derive(Debug, Clone)]
pub struct StreamedDetection {
    pub check: DetectionCheck,
    pub position: (usize, usize),
    pub detection: ContentFilterDetection,
}

// The first match of each category check in a dropped prefix; the scan keeps them in the race
#[derive(Debug, Clone, Default)]
pub struct CarriedMatches {
    pub conditional: Option<CategoryMatch>,
    pub phrase: Option<CategoryMatch>,
    pub keyword: Option<CategoryMatch>,
}

// LiteLLM's _StreamedChoiceState: the buffer window one streamed choice carries between chunks
#[derive(Debug, Clone, Default)]
pub struct StreamedChoiceState {
    pub buffered_text: String,
    pub yielded_masked_text_len: usize,
    pub committed_detections: Vec<StreamedDetection>,
    pub latest_detections: Vec<StreamedDetection>,
    pub next_trim_len: usize,
    // Python's buffer still holds what the dropped prefix had; these stand in for it
    pub seen_exception_phrases: Vec<String>,
    pub open_sentence: String,
    pub carried_matches: CarriedMatches,
}
