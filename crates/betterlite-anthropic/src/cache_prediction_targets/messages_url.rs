use crate::credentials::AnthropicSettings;

pub const MESSAGES_PATH: &str = "/v1/messages";

pub trait MessagesUrl: Sync {
    fn messages_url(&self, api_base: Option<&str>) -> String;
}

// Ports the get_complete_url half of _messages_url: the deployment's base, then the one the settings resolved
impl MessagesUrl for AnthropicSettings {
    fn messages_url(&self, api_base: Option<&str>) -> String {
        let base = api_base
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.api_base);
        if base.ends_with(MESSAGES_PATH) {
            return base.to_owned();
        }
        format!("{base}{MESSAGES_PATH}")
    }
}
