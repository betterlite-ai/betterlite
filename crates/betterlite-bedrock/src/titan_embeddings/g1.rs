use betterlite_prompts::arguments::json::{Json, Object};

// G1 has no mapping step: LiteLLM returns the optional record unchanged and reads no openai parameter
pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &[];

// AmazonTitanG1Config._transform_request sends only the text and ignores every inference parameter
#[must_use]
pub fn transform_request(input: &str) -> Object {
    let mut request = Object::new();
    request.insert(String::from("inputText"), Json::Text(input.to_owned()));
    request
}
