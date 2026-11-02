use std::env::consts::{ARCH, OS};

use betterlite_config::sections::chatgpt_headers::Settings as HeaderSettings;
use nix::sys::utsname::uname;
use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;

pub mod session;

pub const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
pub const DEFAULT_USER_AGENT: &str = "codex_cli_rs/0.0.0 (Unknown 0; unknown) unknown";
const COMPATIBILITY_VERSION: &str = "1.105.1";
const CHATGPT_DEFAULT_INSTRUCTIONS: &str = include_str!("default_instructions.txt");

// reads the real OS, kernel release and architecture so unit tests can inject deterministic values
pub trait PlatformEnvironment: Send + Sync {
    fn os_type(&self) -> String;
    fn os_release(&self) -> String;
    fn arch(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemPlatformEnvironment;

impl PlatformEnvironment for SystemPlatformEnvironment {
    fn os_type(&self) -> String {
        match OS {
            "macos" => "Darwin",
            "linux" => "Linux",
            "windows" => "Windows",
            other => other,
        }
        .to_owned()
    }

    // mirrors Python's platform.release(), which reads the same uname(2) release field
    fn os_release(&self) -> String {
        let Ok(info) = uname() else {
            return "0".to_owned();
        };
        let Some(release) = info.release().to_str() else {
            return "0".to_owned();
        };
        if release.is_empty() {
            "0".to_owned()
        } else {
            release.to_owned()
        }
    }

    fn arch(&self) -> String {
        ARCH.to_owned()
    }
}

fn safe_header_value(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    value
        .chars()
        .map(|letter| {
            if (32..=126).contains(&u32::from(letter)) {
                letter
            } else {
                '_'
            }
        })
        .collect()
}

#[must_use]
pub fn get_chatgpt_originator(settings: &HeaderSettings) -> String {
    let originator = settings
        .originator_override
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_ORIGINATOR);
    let safe = safe_header_value(originator);
    if safe.is_empty() {
        DEFAULT_ORIGINATOR.to_owned()
    } else {
        safe
    }
}

#[must_use]
pub fn get_chatgpt_user_agent(
    settings: &HeaderSettings,
    platform: &dyn PlatformEnvironment,
    originator: &str,
) -> String {
    if let Some(override_value) = settings
        .user_agent_override
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let safe = safe_header_value(override_value);
        return if safe.is_empty() {
            DEFAULT_USER_AGENT.to_owned()
        } else {
            safe
        };
    }
    let trimmed_suffix = settings.user_agent_suffix.as_deref().unwrap_or("").trim();
    let suffix = if trimmed_suffix.is_empty() {
        String::new()
    } else {
        format!(" ({trimmed_suffix})")
    };
    let candidate = format!(
        "{originator}/{COMPATIBILITY_VERSION} ({} {}; {}) {}{suffix}",
        platform.os_type(),
        platform.os_release(),
        platform.arch(),
        settings.terminal,
    );
    let safe = safe_header_value(&candidate);
    if safe.is_empty() {
        DEFAULT_USER_AGENT.to_owned()
    } else {
        safe
    }
}

#[must_use]
pub fn get_chatgpt_default_headers(
    settings: &HeaderSettings,
    platform: &dyn PlatformEnvironment,
    access_token: &SecretString,
    account_id: Option<&str>,
    session_id: Option<&str>,
) -> Headers {
    let originator = get_chatgpt_originator(settings);
    let user_agent = get_chatgpt_user_agent(settings, platform, &originator);
    let mut headers = Headers::new();
    headers.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", access_token.expose_secret()),
    );
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    headers.insert("accept".to_owned(), "text/event-stream".to_owned());
    headers.insert("originator".to_owned(), originator);
    headers.insert("user-agent".to_owned(), user_agent);
    if let Some(truthy_session_id) = session_id.filter(|value| !value.is_empty()) {
        headers.insert("session_id".to_owned(), truthy_session_id.to_owned());
    }
    if let Some(truthy_account_id) = account_id.filter(|value| !value.is_empty()) {
        headers.insert(
            "ChatGPT-Account-Id".to_owned(),
            truthy_account_id.to_owned(),
        );
    }
    headers
}

#[must_use]
pub fn get_chatgpt_instructions(settings: &HeaderSettings) -> &str {
    settings
        .default_instructions_override
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(CHATGPT_DEFAULT_INSTRUCTIONS)
}

mod tests;
