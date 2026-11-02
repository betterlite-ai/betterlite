#![cfg(test)]

use std::collections::BTreeMap;

use super::{ChatGptHeaderEnvironment, resolve};

#[test]
fn chatgpt_header_env_loads() {
    let lookup: BTreeMap<String, String> = [
        ("CHATGPT_ORIGINATOR", "my_cli"),
        ("CHATGPT_USER_AGENT", "my_cli/9.9.9"),
        ("CHATGPT_USER_AGENT_SUFFIX", "  custom build  "),
        ("CHATGPT_DEFAULT_INSTRUCTIONS", "Stay in character."),
        ("TERM_PROGRAM", "iTerm.app"),
        ("TERM_PROGRAM_VERSION", "3.5.0"),
        ("WEZTERM_VERSION", "20240203"),
        ("ITERM_SESSION_ID", "w0t0p0"),
        ("ITERM_PROFILE", "Default"),
        ("ITERM_PROFILE_NAME", "Default"),
        ("TERM_SESSION_ID", "abc"),
        ("KITTY_WINDOW_ID", "1"),
        ("TERM", "xterm-256color"),
        ("ALACRITTY_SOCKET", "/tmp/alacritty.sock"),
        ("KONSOLE_VERSION", "230400"),
        ("GNOME_TERMINAL_SCREEN", "/org/gnome/Terminal/screen/1"),
        ("VTE_VERSION", "6800"),
        ("WT_SESSION", "abc-def"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();

    assert_eq!(
        resolve(&lookup),
        ChatGptHeaderEnvironment {
            originator_override: Some("my_cli".to_owned()),
            user_agent_override: Some("my_cli/9.9.9".to_owned()),
            user_agent_suffix: Some("  custom build  ".to_owned()),
            default_instructions_override: Some("Stay in character.".to_owned()),
            terminal: "iTerm.app/3.5.0".to_owned(),
        }
    );
}

#[test]
fn chatgpt_header_env_loads_defaults_to_unset() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    assert_eq!(
        resolve(&lookup),
        ChatGptHeaderEnvironment {
            originator_override: None,
            user_agent_override: None,
            user_agent_suffix: None,
            default_instructions_override: None,
            terminal: "unknown".to_owned(),
        }
    );
}
