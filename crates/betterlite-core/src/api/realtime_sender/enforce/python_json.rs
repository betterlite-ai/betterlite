use std::str;

// Python's json.loads reads an unpaired surrogate escape that serde_json refuses
const SURROGATE_REPLACEMENT: &[u8] = b"\\uFFFD";
const SURROGATE_FIRST: u16 = 0xD800;
const SURROGATE_LAST: u16 = 0xDFFF;

enum Escape {
    // A \uXXXX escape spelling a surrogate: Python decodes it, and serde_json refuses an unpaired one
    Surrogate(usize),
    // Any other escape, whose length in bytes keeps the scan aligned when both decoders read it or both refuse it
    Plain(usize),
}

fn escape(bytes: &[u8], index: usize) -> Escape {
    let rest = bytes.get(index..).unwrap_or_default();
    let Some(next) = rest.get(1) else {
        return Escape::Plain(1);
    };
    if next != &b'u' {
        return Escape::Plain(2);
    }
    let Some(value) = rest
        .get(2..6)
        .and_then(|digits| str::from_utf8(digits).ok())
        .and_then(|text| u16::from_str_radix(text, 16).ok())
    else {
        return Escape::Plain(2);
    };
    if (SURROGATE_FIRST..=SURROGATE_LAST).contains(&value) {
        return Escape::Surrogate(6);
    }
    Escape::Plain(6)
}

// A constant is a value only after a value separator, so a token glued to a number is one Python's parser refuses
fn constant(bytes: &[u8], index: usize) -> Option<usize> {
    let rest = bytes.get(index..)?;
    let length = ["-Infinity", "Infinity", "NaN"]
        .into_iter()
        .find(|token| rest.starts_with(token.as_bytes()))?
        .len();
    let preceding = bytes
        .get(..index)?
        .iter()
        .rev()
        .find(|byte| !byte.is_ascii_whitespace());
    if preceding.is_some_and(|byte| !matches!(byte, b':' | b'[' | b',')) {
        return None;
    }
    match bytes.get(index.saturating_add(length)).copied() {
        None | Some(b',' | b']' | b'}') => Some(length),
        Some(byte) => byte.is_ascii_whitespace().then_some(length),
    }
}

// A replacement inside a string leaves the string valid, and an escape outside one stays invalid for both parsers
pub(super) fn normalize_extensions(message: &str) -> Option<String> {
    let bytes = message.as_bytes();
    let mut output = Vec::with_capacity(message.len());
    let mut index = 0_usize;
    let mut changed = false;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'\\' {
            match escape(bytes, index) {
                Escape::Surrogate(length) => {
                    output.extend_from_slice(SURROGATE_REPLACEMENT);
                    changed = true;
                    index = index.saturating_add(length);
                }
                Escape::Plain(length) => {
                    let text = bytes
                        .get(index..index.saturating_add(length))
                        .unwrap_or_default();
                    output.extend_from_slice(text);
                    index = index.saturating_add(length);
                }
            }
            continue;
        }
        if let Some(length) = constant(bytes, index) {
            output.push(b'0');
            changed = true;
            index = index.saturating_add(length);
            continue;
        }
        output.push(byte);
        index = index.saturating_add(1);
    }
    if !changed {
        return None;
    }
    String::from_utf8(output).ok()
}
