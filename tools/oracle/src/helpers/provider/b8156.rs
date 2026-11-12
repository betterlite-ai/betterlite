use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::message_content::Message;
use betterlite_provider::ollama_generate::config::OllamaGenerateConfig;
use betterlite_provider::ollama_generate::request::RequestParams;
use indexmap::IndexMap;
use serde::Deserialize;

use self::embedding::embedding_cases;
use self::ops::{param_cases, reply_cases, request_cases, stream_cases};
use crate::helpers::Helper;
use crate::helpers::HelperResult;

mod embedding;
mod ops;

#[derive(Deserialize)]
pub(crate) struct ParamsInput {
    #[serde(default)]
    config: OllamaGenerateConfig,
    #[serde(default)]
    cases: Vec<ParamCase>,
    #[serde(default)]
    urls: Vec<Option<String>>,
}

#[derive(Deserialize)]
pub(crate) struct ParamCase {
    pub(super) model: String,
    pub(super) non_default: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) optional: IndexMap<String, Json>,
}

#[derive(Deserialize)]
pub(crate) struct RequestCase {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) litellm_params: RequestParams,
}

#[derive(Deserialize)]
pub(crate) struct ReplyCase {
    pub(super) model: String,
    pub(super) body: String,
    #[serde(default)]
    pub(super) request_data: IndexMap<String, Json>,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EncoderKind {
    #[default]
    None,
    Words,
    Failing,
}

#[derive(Deserialize)]
pub(crate) struct EmbeddingCase {
    pub(super) api_base: String,
    pub(super) model: String,
    pub(super) prompts: Vec<String>,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) encoder: EncoderKind,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Params(Box<ParamsInput>),
    Request {
        #[serde(default)]
        custom_prompt_dict: BTreeMap<String, CustomPromptTemplate>,
        #[serde(default)]
        requests: Vec<RequestCase>,
    },
    Reply {
        #[serde(default)]
        replies: Vec<ReplyCase>,
    },
    Stream {
        #[serde(default)]
        lines: Vec<String>,
    },
    Embedding {
        #[serde(default)]
        cases: Vec<EmbeddingCase>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Params(params) => Ok(param_cases(&params.config, &params.cases, &params.urls)),
            Input::Request {
                custom_prompt_dict,
                requests,
            } => Ok(request_cases(custom_prompt_dict, requests)),
            Input::Reply { replies } => reply_cases(replies),
            Input::Stream { lines } => Ok(stream_cases(lines)),
            Input::Embedding { cases } => embedding_cases(cases),
        }
    }
}
