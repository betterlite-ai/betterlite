use betterlite_config::sections::content_filter_records::ContentFilterDetection;

mod tests;

// LiteLLM's _StreamedChoiceState: the buffer window one streamed choice carries between chunks
#[derive(Debug, Clone, Default)]
pub struct StreamedChoiceState {
    pub buffered_text: String,
    pub yielded_masked_text_len: usize,
    pub committed_detections: Vec<ContentFilterDetection>,
    pub latest_detections: Vec<ContentFilterDetection>,
    pub next_trim_len: usize,
    // Python's buffer still holds what the dropped prefix had; these flags stand in for it
    pub seen_exception_phrases: Vec<String>,
    pub open_sentence_words: Vec<String>,
}
