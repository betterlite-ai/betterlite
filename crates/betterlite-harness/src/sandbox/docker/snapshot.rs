use std::collections::BTreeMap;

use super::DockerSandbox;
use super::error::DockerSandboxError;
use super::process::decode_utf8;
use crate::errors::HarnessError;
use crate::sandbox::Sandbox as _;
use crate::snapshots::{MAX_FILE_BYTES, SKIP_DIRS};

const SHA256_HEX_CHARS: usize = 64;
const SHA256_NAME_START: usize = 66;

pub(super) fn find_script() -> String {
    let mut names = SKIP_DIRS;
    names.sort_unstable();
    let prune = names
        .iter()
        .map(|name| format!("-name '{name}'"))
        .collect::<Vec<String>>()
        .join(" -o ");
    format!(
        "cd \"$1\" && find . -type d \\( {prune} \\) -prune -o -type f -size -{}c -exec sha256sum {{}} +",
        MAX_FILE_BYTES.saturating_add(1)
    )
}

pub(super) async fn capture(
    sandbox: &DockerSandbox,
) -> Result<BTreeMap<String, String>, HarnessError> {
    let command = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        find_script(),
        "sh".to_owned(),
        sandbox.workdir().to_owned(),
    ];
    let (code, stdout, stderr) = sandbox
        .exec_capture(command, None, super::MAX_HELPER_OUTPUT_BYTES)
        .await?;
    if code != 0_i32 {
        return Err(HarnessError::from(DockerSandboxError::Snapshot {
            stderr: decode_utf8(&stderr).trim().to_owned(),
        }));
    }
    Ok(parse_sha256sum(&stdout))
}

#[must_use]
pub fn parse_sha256sum(output: &[u8]) -> BTreeMap<String, String> {
    let text = decode_utf8(output);
    let mut hashes = BTreeMap::new();
    for line in splitlines(&text) {
        if let Some((key, value)) = parse_line(line) {
            hashes.insert(key, value);
        }
    }
    hashes
}

fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        // Python's str.splitlines breaks on these, where str::lines only knows \n
        let boundary = matches!(
            character,
            '\n' | '\u{b}'
                | '\u{c}'
                | '\r'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !boundary {
            continue;
        }
        lines.push(text.get(start..index).unwrap_or_default());
        if character == '\r' {
            if let Some((next_index, '\n')) = characters.peek() {
                start = next_index.saturating_add(1);
                let _skipped = characters.next();
            } else {
                start = index.saturating_add(1);
            }
        } else {
            start = index.saturating_add(character.len_utf8());
        }
    }
    if start < text.len() {
        lines.push(text.get(start..).unwrap_or_default());
    }
    lines
}

fn parse_line(line: &str) -> Option<(String, String)> {
    if line.chars().count() <= SHA256_NAME_START {
        return None;
    }
    let value_end = line
        .char_indices()
        .nth(SHA256_HEX_CHARS)
        .map(|(index, _character)| index)?;
    let key_start = line
        .char_indices()
        .nth(SHA256_NAME_START)
        .map(|(index, _character)| index)?;
    let name = line.get(key_start..)?;
    let key = name.strip_prefix("./").unwrap_or(name);
    let value = line.get(..value_end)?;
    Some((key.to_owned(), value.to_owned()))
}
