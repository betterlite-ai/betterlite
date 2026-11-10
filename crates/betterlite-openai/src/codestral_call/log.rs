use secrecy::SecretString;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::codestral_records::reply::PostCallLogger;
use crate::openai_base_config::Headers;

pub struct PreCall<'call> {
    pub input: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub body: &'call Map<String, Value>,
    pub headers: &'call Headers,
    pub api_base: &'call str,
    pub acompletion: bool,
}

// make_call hands its callbacks the Python line iterator itself, which a native caller cannot share
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginalResponse {
    Lines,
}

pub struct StreamPostCall<'call> {
    pub input: &'call [Value],
    pub api_key: &'call str,
    pub original_response: OriginalResponse,
    // make_call logs json.dumps(data), a string, where the pre-call log holds the object
    pub request_data: &'call str,
}

pub trait CallLogger: PostCallLogger {
    fn pre_call(&mut self, call: PreCall<'_>);

    fn stream_post_call(&mut self, call: StreamPostCall<'_>);
}
