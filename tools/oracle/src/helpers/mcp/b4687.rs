use crate::helpers::{Helper, HelperResult};
use betterlite_mcp::byok_authorization_html::{AuthorizePage, build_authorize_html};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub(crate) struct Input {
    server_name: String,
    server_initial: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: String,
    server_id: String,
    access_items: Vec<String>,
    help_url: String,
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let html = build_authorize_html(&AuthorizePage {
            server_name: input.server_name.clone(),
            server_initial: input.server_initial.clone(),
            client_id: input.client_id.clone(),
            redirect_uri: input.redirect_uri.clone(),
            code_challenge: input.code_challenge.clone(),
            code_challenge_method: input.code_challenge_method.clone(),
            state: input.state.clone(),
            server_id: input.server_id.clone(),
            access_items: input.access_items.clone(),
            help_url: input.help_url.clone(),
        });
        let oauth_values_escaped = [
            (&input.client_id, "client_id"),
            (&input.redirect_uri, "redirect_uri"),
            (&input.code_challenge, "code_challenge"),
            (&input.code_challenge_method, "code_challenge_method"),
            (&input.state, "state"),
            (&input.server_id, "server_id"),
        ]
        .iter()
        .all(|(value, name)| {
            html.contains(&format!("name=\"{name}\" value=\"{}\"", escape(value)))
        });
        Ok(json!({
            "steps": [html.contains("id=\"s1\""), html.contains("id=\"s2\"")],
            "api_key": html.contains("name=\"api_key\""),
            "duration": html.contains("value=\"until_revoked\"") && html.contains("selDur('24h'"),
            "access_items": input.access_items.iter().all(|item| html.contains(&escape(item))),
            "help_link": html.contains("class=\"help-link\""),
            "oauth_values_escaped": oauth_values_escaped
        }))
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

mod tests;
