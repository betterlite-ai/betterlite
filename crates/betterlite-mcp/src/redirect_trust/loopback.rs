use super::shape::{RedirectUriRejected, is_loopback_redirect_host, parse_redirect_uri};

// LiteLLM's validate_loopback_redirect_uri: the callback must be a loopback listener.
pub fn validate_loopback_redirect_uri(redirect_uri: &str) -> Result<(), RedirectUriRejected> {
    let parsed = parse_redirect_uri(redirect_uri)?;
    if parsed.scheme != "http" && parsed.scheme != "https" {
        return Err(RedirectUriRejected {
            description: format!(
                "redirect_uri scheme '{}' is not allowed; use http or https.",
                parsed.scheme
            ),
            hint: None,
            redirect_uri: None,
        });
    }
    if !parsed.fragment.is_empty() {
        return Err(RedirectUriRejected {
            description: "redirect_uri must not contain a URL fragment (#...).".to_owned(),
            hint: None,
            redirect_uri: None,
        });
    }
    if is_loopback_redirect_host(&parsed) {
        return Ok(());
    }
    Err(RedirectUriRejected {
        description: "redirect_uri must use a loopback host (localhost or 127.0.0.0/8).".to_owned(),
        hint: Some(
            "Native MCP clients should register a callback on http://127.0.0.1:<port>/..."
                .to_owned(),
        ),
        redirect_uri: None,
    })
}
