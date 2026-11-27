// Starlette's cookie_parser: chunks split on ';', each at the first '=', trimmed, and the last token entry wins.
#[must_use]
pub fn session_token_cookie(header: Option<&str>) -> Option<String> {
    let header = header?;
    let mut token: Option<String> = None;
    for chunk in header.split(';') {
        let (key, value) = chunk.split_once('=').unwrap_or(("", chunk));
        let key = key.trim();
        let value = value.trim();
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
        if character != '\\' {
            decoded.push(character);
            index = index.saturating_add(1);
            continue;
        }
        let escaped = characters.get(index.saturating_add(1)).copied();
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
            decoded.push(character);
            index = index.saturating_add(4);
            continue;
        }
        match escaped {
            Some(character) if character != '\n' => {
                decoded.push(character);
                index = index.saturating_add(2);
            }
            _ => {
                decoded.push('\\');
                index = index.saturating_add(1);
            }
        }
    }
    decoded
}
