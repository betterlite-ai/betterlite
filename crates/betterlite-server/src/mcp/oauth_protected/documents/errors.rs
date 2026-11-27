#[cfg(test)]
mod tests;

use axum::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{detail}")]
pub(crate) struct DiscoveryFailure {
    pub(crate) status: StatusCode,
    pub(crate) detail: String,
}

pub(crate) fn not_oauth_protected(name: &str) -> DiscoveryFailure {
    DiscoveryFailure {
        status: StatusCode::NOT_FOUND,
        detail: format!("MCP server '{name}' is not an OAuth-protected resource"),
    }
}

pub(crate) fn upstream_unavailable(name: &str) -> DiscoveryFailure {
    DiscoveryFailure {
        status: StatusCode::BAD_GATEWAY,
        detail: format!(
            "Upstream oauth-protected-resource metadata unavailable for MCP server {}",
            python_repr(name)
        ),
    }
}

pub(crate) fn upstream_fetch_failed(name: &str) -> DiscoveryFailure {
    DiscoveryFailure {
        status: StatusCode::BAD_GATEWAY,
        detail: format!(
            "Failed to fetch upstream oauth-protected-resource metadata for MCP server {}",
            python_repr(name)
        ),
    }
}

pub(crate) fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(text.len().saturating_add(2));
    rendered.push(quote);
    for character in text.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            escaped if escaped == quote => {
                rendered.push('\\');
                rendered.push(escaped);
            }
            other => rendered.push(other),
        }
    }
    rendered.push(quote);
    rendered
}
