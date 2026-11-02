#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::chatgpt_headers::Settings;

fn settings_from_full_environment() -> Settings {
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

    ConfigLoader { lookup }
        .load("")
        .expect("a document with no chatgpt_headers keys still loads")
        .sections
        .chatgpt_headers
}

#[test]
fn chatgpt_header_env_loads() {
    let settings = settings_from_full_environment();

    assert_eq!(settings.originator_override.as_deref(), Some("my_cli"));
    assert_eq!(
        settings.user_agent_override.as_deref(),
        Some("my_cli/9.9.9")
    );
    assert_eq!(
        settings.user_agent_suffix.as_deref(),
        Some("  custom build  ")
    );
    assert_eq!(
        settings.default_instructions_override.as_deref(),
        Some("Stay in character.")
    );
    assert_eq!(settings.terminal, "iTerm.app/3.5.0");
}

#[test]
fn chatgpt_header_env_defaults_to_unset() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .expect("a document with no chatgpt_headers keys still loads");
    let settings = config.sections.chatgpt_headers;

    assert_eq!(settings.originator_override, None);
    assert_eq!(settings.user_agent_override, None);
    assert_eq!(settings.user_agent_suffix, None);
    assert_eq!(settings.default_instructions_override, None);
    assert_eq!(settings.terminal, "unknown");
}
