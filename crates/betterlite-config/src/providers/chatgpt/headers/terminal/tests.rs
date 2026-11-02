#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::resolve;

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case(&[("TERM_PROGRAM", "iTerm.app"), ("TERM_PROGRAM_VERSION", "3.5"), ("WEZTERM_VERSION", "20240203")], "iTerm.app/3.5")]
#[case(&[("WEZTERM_VERSION", "")], "WezTerm")]
#[case(&[("WEZTERM_VERSION", "20240203")], "WezTerm/20240203")]
#[case(&[("ITERM_SESSION_ID", "w0t0p0")], "iTerm.app")]
#[case(&[("ITERM_PROFILE", "Default")], "iTerm.app")]
#[case(&[("TERM_SESSION_ID", "abc")], "Apple_Terminal")]
#[case(&[("KITTY_WINDOW_ID", "1")], "kitty")]
#[case(&[("TERM", "xterm-kitty")], "kitty")]
#[case(&[("ALACRITTY_SOCKET", "/tmp/sock")], "Alacritty")]
#[case(&[("TERM", "alacritty")], "Alacritty")]
#[case(&[("KONSOLE_VERSION", "")], "Konsole")]
#[case(&[("GNOME_TERMINAL_SCREEN", "/org/gnome/Terminal/screen/1")], "gnome-terminal")]
#[case(&[("VTE_VERSION", "")], "VTE")]
#[case(&[("WT_SESSION", "abc")], "WindowsTerminal")]
#[case(&[("TERM", "xterm-256color")], "xterm-256color")]
#[case(&[], "unknown")]
fn resolves_terminal_precedence(#[case] env: &[(&str, &str)], #[case] expected: &str) {
    assert_eq!(resolve(&lookup(env)), expected);
}
