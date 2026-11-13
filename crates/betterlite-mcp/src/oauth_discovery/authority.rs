use crate::oauth_resource::python_url::{self, netloc};

#[must_use]
pub fn is_same_authority_metadata_url(url: &str, server_url: &str) -> bool {
    let Ok(target) = python_url::parse(url) else {
        return false;
    };
    let Ok(base) = python_url::parse(server_url) else {
        return false;
    };
    if !matches!(target.scheme.as_str(), "http" | "https") {
        return false;
    }
    let Some(target_host) = netloc::hostname(&target.netloc) else {
        return false;
    };
    let Ok(target_explicit_port) = netloc::port(&target.netloc) else {
        return false;
    };
    let Ok(base_explicit_port) = netloc::port(&base.netloc) else {
        return false;
    };
    let target_port = target_explicit_port.unwrap_or_else(|| default_port(&target.scheme));
    let base_port = base_explicit_port.unwrap_or_else(|| default_port(&base.scheme));
    base.scheme == target.scheme
        && netloc::hostname(&base.netloc).is_some_and(|host| host == target_host)
        && base_port == target_port
}

fn default_port(scheme: &str) -> u16 {
    if scheme == "https" { 443 } else { 80 }
}
