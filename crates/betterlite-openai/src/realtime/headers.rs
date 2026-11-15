use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;

pub const OPENAI_BETA_HEADER: &str = "OpenAI-Beta";
pub const REALTIME_BETA_VALUE: &str = "realtime=v1";

#[must_use]
pub fn additional_headers(api_key: &SecretString, beta_realtime: bool) -> Headers {
    let mut headers = Headers::from([(
        String::from("Authorization"),
        format!("Bearer {}", api_key.expose_secret()),
    )]);
    if beta_realtime {
        headers.insert(
            String::from(OPENAI_BETA_HEADER),
            String::from(REALTIME_BETA_VALUE),
        );
    }
    headers
}
