#![cfg(test)]

use std::env;
use std::error::Error as _;
use std::path::Path;

use rstest::rstest;

use super::{install_statusline_script, is_our_statusline, statusline_command};
use crate::claude_settings::error::ClaudeSettingsError;

const COMMAND: &str = "'/usr/local/bin/lite' statusline";
const CMD_COMMAND: &str = "\"C:\\bin\\lite.exe\" \"statusline\"";

#[test]
fn statusline_command_leaves_a_plain_posix_path_unquoted() {
    assert_eq!(
        statusline_command(Path::new("/usr/local/bin/lite"), "linux").unwrap(),
        "/usr/local/bin/lite statusline"
    );
}

#[test]
fn statusline_command_single_quotes_a_posix_path_with_a_space() {
    assert_eq!(
        statusline_command(Path::new("/opt/my tools/lite"), "linux").unwrap(),
        "'/opt/my tools/lite' statusline"
    );
}

#[test]
fn statusline_command_quotes_a_posix_path_with_an_apostrophe() {
    assert_eq!(
        statusline_command(Path::new("/opt/it's/lite"), "linux").unwrap(),
        "\"/opt/it's/lite\" statusline"
    );
}

#[test]
fn statusline_command_quotes_an_empty_posix_path() {
    assert_eq!(
        statusline_command(Path::new(""), "linux").unwrap(),
        "'' statusline"
    );
}

#[rstest]
#[case::win32("win32")]
#[case::windows("windows")]
fn statusline_command_double_quotes_the_path_for_cmd(#[case] platform: &str) {
    assert_eq!(
        statusline_command(
            Path::new("C:\\Program Files\\betterlite\\lite.exe"),
            platform
        )
        .unwrap(),
        "\"C:\\Program Files\\betterlite\\lite.exe\" \"statusline\""
    );
}

#[test]
fn install_statusline_script_resolves_the_running_executable() {
    let executable = env::current_exe().unwrap();
    let expected = statusline_command(&executable, env::consts::OS).unwrap();
    assert_eq!(install_statusline_script(None).unwrap(), expected);
}

#[test]
fn install_statusline_script_takes_an_injected_path() {
    let path = Path::new("/opt/my tools/lite");
    let expected = statusline_command(path, env::consts::OS).unwrap();
    assert_eq!(install_statusline_script(Some(path)).unwrap(), expected);
    assert!(expected.contains("/opt/my tools/lite"));
}

#[test]
fn statusline_command_reports_a_posix_path_that_cannot_be_quoted() {
    let error = statusline_command(Path::new("/opt/a\0b"), "linux").unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Quote { .. }));
    assert!(error.source().is_some());
}

#[cfg(unix)]
#[test]
fn statusline_command_refuses_a_path_that_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let path = Path::new(OsStr::from_bytes(b"/opt/\xff/lite"));
    let error = statusline_command(path, "linux").unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::NonUtf8Path { .. }));
}

#[rstest]
#[case::same_binary("/opt/old/lite statusline", true)]
#[case::quoted_binary("'/opt/my tools/lite' statusline", true)]
#[case::bare_binary("lite statusline", true)]
#[case::ccusage_via_bun("bun x ccusage statusline", false)]
#[case::ccusage_via_npx("npx -y ccusage statusline", false)]
#[case::our_script("python3 /home/tester/.litellm/statusline.py", false)]
#[case::another_binary("/usr/bin/other statusline", false)]
fn is_our_statusline_matches_a_two_word_command_for_the_same_binary(
    #[case] existing: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_our_statusline(existing, COMMAND), expected);
}

#[rstest]
#[case::same_binary("\"C:\\old\\lite.exe\" \"statusline\"", true)]
#[case::quoted_space("\"C:\\Program Files\\betterlite\\lite.exe\" \"statusline\"", true)]
#[case::another_binary("\"C:\\bin\\other.exe\" \"statusline\"", false)]
#[case::ccusage("\"C:\\bin\\bun.exe\" x ccusage statusline", false)]
fn is_our_statusline_parses_cmd_quoting(#[case] existing: &str, #[case] expected: bool) {
    assert_eq!(is_our_statusline(existing, CMD_COMMAND), expected);
}
