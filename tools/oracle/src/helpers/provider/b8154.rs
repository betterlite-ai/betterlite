use std::collections::BTreeMap;

use betterlite_openai::embedding::EmbeddingInput;
use serde::Deserialize;

use self::script::EmbeddingReplyCase;
use self::script::MapCase;
use self::script::ReplyCase;
use self::script::RequestCase;
use self::script::SplitCase;
use self::script::UrlCase;
use crate::helpers::Helper;
use crate::helpers::HelperResult;

mod ops;
mod script;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    LlmType {
        bases: Vec<String>,
        urls: Vec<UrlCase>,
        map: Vec<MapCase>,
    },
    GenerateRequest {
        #[serde(default)]
        env: BTreeMap<String, String>,
        requests: Vec<RequestCase>,
    },
    InferRequest {
        requests: Vec<RequestCase>,
    },
    GenerateReply {
        replies: Vec<ReplyCase>,
    },
    InferReply {
        replies: Vec<ReplyCase>,
    },
    StreamLine {
        lines: Vec<String>,
    },
    Embedding {
        inputs: Vec<EmbeddingInput>,
        replies: Vec<EmbeddingReplyCase>,
        split: Vec<SplitCase>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::LlmType { bases, urls, map } => Ok(ops::llm_type(bases, urls, map)),
            Input::GenerateRequest { env, requests } => ops::generate_requests(env, requests),
            Input::InferRequest { requests } => Ok(ops::infer_requests(requests)),
            Input::GenerateReply { replies } => Ok(ops::generate_replies(replies)),
            Input::InferReply { replies } => Ok(ops::infer_replies(replies)),
            Input::StreamLine { lines } => Ok(ops::stream_lines(lines)),
            Input::Embedding {
                inputs,
                replies,
                split,
            } => Ok(ops::embedding(inputs, replies, split)),
        }
    }
}
