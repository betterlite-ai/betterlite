use betterlite_tokenizer::count::error::CountError;
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;

// The passthrough reply counts the joined text without loading a tokenizer, as the oracle's standin counts its length.
pub(super) struct CharacterCounter;

impl TextTokenCounter for CharacterCounter {
    fn count_text(&self, _model: &str, text: &str) -> Result<u32, CountError> {
        u32::try_from(text.chars().count()).map_err(|_error| CountError::Overflow)
    }
}
