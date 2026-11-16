use betterlite_prompts::arguments::json::Object;

use crate::straiker::v3::request_header::inbound_header;
use crate::straiker::v3::{CLIENT_BY_USER_AGENT, GATEWAY_NAME};

// The Straiker client value and the agent display name for a User-Agent this gateway recognises.
#[must_use]
pub fn client_from_user_agent(request_data: &Object) -> Option<(String, String)> {
    let user_agent = inbound_header(request_data, "user-agent")?.to_lowercase();
    CLIENT_BY_USER_AGENT
        .iter()
        .find_map(|&(prefix, client, display)| {
            user_agent
                .starts_with(prefix)
                .then(|| (client.to_owned(), format!("{display} ({GATEWAY_NAME})")))
        })
}

mod tests;
