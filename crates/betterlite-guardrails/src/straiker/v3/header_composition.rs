use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

use crate::straiker::v3::client_recognition::client_from_user_agent;
use crate::straiker::v3::request_header::inbound_header;
use crate::straiker::v3::{AGENT_HEADER, CLIENT_HEADER, FORMAT_HEADER, SESSION_HEADER};

// The route's agent_ref outranks the caller's own header: the header is caller-supplied, and honouring it over a pinned route would let any key file its traffic under another application's agent.
#[must_use]
pub fn headers(
    request_data: &Object,
    agent_ref: Option<&str>,
    client: Option<&str>,
    format_hint: Option<&str>,
) -> IndexMap<&'static str, String> {
    let recognised = client_from_user_agent(request_data);
    let agent = agent_ref
        .map(str::to_owned)
        .filter(|name| !name.is_empty())
        .or_else(|| inbound_header(request_data, AGENT_HEADER))
        .or_else(|| recognised.as_ref().map(|(_, display)| display.clone()));
    let named_client = client
        .map(str::to_owned)
        .filter(|name| !name.is_empty())
        .or_else(|| recognised.map(|(recognised_client, _)| recognised_client));
    [
        (SESSION_HEADER, inbound_header(request_data, SESSION_HEADER)),
        (AGENT_HEADER, agent),
        (CLIENT_HEADER, named_client),
        (FORMAT_HEADER, format_hint.map(str::to_owned)),
    ]
    .into_iter()
    .filter_map(|(name, value)| Some((name, value.filter(|text| !text.is_empty())?)))
    .collect()
}

mod tests;
