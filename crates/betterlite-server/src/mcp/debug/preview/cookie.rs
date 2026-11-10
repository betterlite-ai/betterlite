use regex::Regex;
use secrecy::SecretString;

// CPython http.cookies _CookiePattern with re.ASCII, expanded because the regex crate has no ASCII flag
const COOKIE_PATTERN: &str = r#"\A[ \t\n\r\f\v]*(?P<key>[0-9A-Za-z_!#%&'~_`><@,:/$*+\-.^|)(?}{=]+?)([ \t\n\r\f\v]*=[ \t\n\r\f\v]*(?P<val>"(?:[^\\"]|\\.)*"|([0-9A-Za-z]{3,6}day|[0-9A-Za-z]{3})[ \t\n\r\f\v][0-9A-Za-z_ \t\n\r\f\v-]{9,11}[ \t\n\r\f\v][0-9:]{8}[ \t\n\r\f\v]GMT|[0-9A-Za-z_!#%&'~_`><@,:/$*+\-.^|)(?}{=\[\]]*))?[ \t\n\r\f\v]*(?:[ \t\n\r\f\v]+|;|\z)"#;
const RESERVED_ATTRIBUTES: [&str; 9] = [
    "comment", "domain", "expires", "httponly", "max-age", "path", "samesite", "secure", "version",
];
const FLAG_ATTRIBUTES: [&str; 2] = ["httponly", "secure"];

// None means the whole header string is not a valid cookie list, so only the raw value stays a secret
pub(super) fn cookie_values(value: &str) -> Option<Vec<SecretString>> {
    let pattern = Regex::new(COOKIE_PATTERN).ok()?;
    let mut remaining = value;
    let mut morsel_seen = false;
    let mut pairs: Vec<(String, String)> = Vec::new();
    while !remaining.is_empty() {
        let Some(captures) = pattern.captures(remaining) else {
            break;
        };
        let consumed = captures.get(0)?.end();
        let key = captures.name("key")?.as_str();
        let lowered = key.to_lowercase();
        let raw_value = captures.name("val").map(|matched| matched.as_str());
        remaining = remaining.get(consumed..)?;
        if key.starts_with('$') {
            continue;
        }
        if RESERVED_ATTRIBUTES.contains(&lowered.as_str()) {
            if !morsel_seen || (raw_value.is_none() && !FLAG_ATTRIBUTES.contains(&lowered.as_str()))
            {
                return None;
            }
            continue;
        }
        record_morsel(&mut pairs, key, raw_value?);
        morsel_seen = true;
    }
    Some(
        pairs
            .into_iter()
            .map(|(_key, quoted)| SecretString::from(quoted))
            .collect(),
    )
}

fn record_morsel(pairs: &mut Vec<(String, String)>, key: &str, value: &str) {
    let unquoted = unquote(value);
    match pairs
        .iter_mut()
        .find(|(existing, _value)| existing.as_str() == key)
    {
        Some(entry) => entry.1 = unquoted,
        None => pairs.push((key.to_owned(), unquoted)),
    }
}

fn unquote(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.to_owned();
    };
    let characters: Vec<char> = inner.chars().collect();
    let mut unquoted = String::with_capacity(inner.len());
    let mut position = 0_usize;
    while let Some(character) = characters.get(position) {
        if *character != '\\' {
            unquoted.push(*character);
            position = position.saturating_add(1);
            continue;
        }
        let octal: String = characters
            .iter()
            .skip(position.saturating_add(1))
            .take(3)
            .collect();
        if is_octal_escape(&octal)
            && let Some(decoded) = u32::from_str_radix(&octal, 8).ok().and_then(char::from_u32)
        {
            unquoted.push(decoded);
            position = position.saturating_add(4);
            continue;
        }
        if let Some(escaped) = characters.get(position.saturating_add(1)) {
            unquoted.push(*escaped);
            position = position.saturating_add(2);
            continue;
        }
        unquoted.push('\\');
        break;
    }
    unquoted
}

fn is_octal_escape(text: &str) -> bool {
    text.len() == 3
        && text
            .chars()
            .next()
            .is_some_and(|leading| ('0'..='3').contains(&leading))
        && text.chars().all(|digit| ('0'..='7').contains(&digit))
}
