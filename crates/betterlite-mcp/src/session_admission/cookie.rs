#[must_use]
pub fn session_token_cookie(header: Option<&str>) -> Option<String> {
    let raw_header = header?;
    let mut token: Option<String> = None;
    for chunk in raw_header.split(';') {
        let (raw_key, raw_value) = chunk.split_once('=').unwrap_or(("", chunk));
        let key = raw_key.trim();
        let value = raw_value.trim();
        if key.is_empty() && value.is_empty() {
            continue;
        }
        if key == "token" {
            token = Some(unquote(value));
        }
    }
    token
}

// http.cookies._unquote: only a fully double-quoted value is decoded, then octal and escaped characters resolve.
fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"') else {
        return value.to_owned();
    };
    let Some(quoted) = inner.strip_suffix('"') else {
        return value.to_owned();
    };
    let characters: Vec<char> = quoted.chars().collect();
    let mut decoded = String::with_capacity(quoted.len());
    let mut index = 0_usize;
    while let Some(character) = characters.get(index).copied() {
        if character == '\\' {
            let (escaped, consumed) = escaped_character(&characters, index);
            decoded.push(escaped);
            index = index.saturating_add(consumed);
            continue;
        }
        decoded.push(character);
        index = index.saturating_add(1);
    }
    decoded
}

// _unquote's escape: three octal digits, any character but a newline, or a dangling backslash.
fn escaped_character(characters: &[char], index: usize) -> (char, usize) {
    let octal = match (
        characters.get(index.saturating_add(1)),
        characters.get(index.saturating_add(2)),
        characters.get(index.saturating_add(3)),
    ) {
        (Some(first @ '0'..='3'), Some(second @ '0'..='7'), Some(third @ '0'..='7')) => {
            let code = u32::from(*first)
                .wrapping_sub(u32::from('0'))
                .wrapping_mul(64)
                .wrapping_add(
                    u32::from(*second)
                        .wrapping_sub(u32::from('0'))
                        .wrapping_mul(8),
                )
                .wrapping_add(u32::from(*third).wrapping_sub(u32::from('0')));
            char::from_u32(code)
        }
        _ => None,
    };
    if let Some(character) = octal {
        return (character, 4);
    }
    match characters.get(index.saturating_add(1)).copied() {
        Some(character) if character != '\n' => (character, 2),
        Some(_) | None => ('\\', 1),
    }
}
