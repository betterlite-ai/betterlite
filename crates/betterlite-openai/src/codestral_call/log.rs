use crate::codestral_records::reply::PostCallLogger;
use crate::openai_base_config::Headers;
use secrecy::SecretString;
use serde_json::{Map, Value};
pub struct PreCall<'call> {
    pub input: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub body: &'call Map<String, Value>,
    pub headers: &'call Headers,
    pub api_base: &'call str,
    pub acompletion: bool,
}
pub struct StreamPostCall<'call> {
    pub input: &'call [Value],
    pub api_key: &'call str,
    pub original_response: &'static str,
    pub request_data: &'call str,
}
pub trait CallLogger: PostCallLogger {
    fn pre_call(&mut self, call: PreCall<'_>);
    fn stream_post_call(&mut self, call: StreamPostCall<'_>);
}
