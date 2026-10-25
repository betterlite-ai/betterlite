use betterlite_guardrails::bedrock_chunking::{bin_pack_bedrock_content, merge_bedrock_guardrail_responses, nearest_whitespace_split_index, split_bedrock_content, BedrockContentChunkResult};
use betterlite_guardrails::bedrock_records::{BedrockContentItem, BedrockGuardrailResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
pub(crate) struct Input { step: Step }
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
enum Step { BinPack { content: Vec<BedrockContentItem>, budget: usize }, Split { content: Vec<BedrockContentItem> }, Nearest { text: String }, Merge { chunks: Vec<Chunk> } }
#[derive(Debug, Deserialize)]
struct Chunk { response: BedrockGuardrailResponse, content: Vec<BedrockContentItem>, fragment_group_size: usize }

fn encode<T: Serialize>(value: T) -> crate::helpers::HelperResult {
    serde_json::to_value(value).map_err(|error| Box::new(crate::helpers::HelperError { class: "SerializationError".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match &input.step {
            Step::BinPack { content, budget } => return encode(bin_pack_bedrock_content(content, *budget)),
            Step::Split { content } => return encode(split_bedrock_content(content)),
            Step::Nearest { text } => json!(nearest_whitespace_split_index(text)),
            Step::Merge { chunks } => {
                let owned = chunks.iter().map(|chunk| BedrockContentChunkResult { response: chunk.response.clone(), content: chunk.content.clone(), fragment_group_size: chunk.fragment_group_size }).collect::<Vec<_>>();
                return encode(merge_bedrock_guardrail_responses(&owned));
            }
        };
        Ok(output)
    }
}
