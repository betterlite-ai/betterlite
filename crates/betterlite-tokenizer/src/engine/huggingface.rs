use tokenizers::Tokenizer;

use crate::engine::error::EngineError;

#[derive(Debug)]
pub struct HuggingFaceEngine {
    tokenizer: Tokenizer,
}

impl HuggingFaceEngine {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EngineError> {
        match Tokenizer::from_bytes(bytes) {
            Ok(tokenizer) => Ok(Self { tokenizer }),
            Err(failure) => Err(EngineError::HuggingFaceLoad(failure)),
        }
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u32>, EngineError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(EngineError::HuggingFaceEncode)?;
        Ok(encoding.get_ids().to_vec())
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String, EngineError> {
        self.tokenizer
            .decode(ids, true)
            .map_err(EngineError::HuggingFaceDecode)
    }
}
