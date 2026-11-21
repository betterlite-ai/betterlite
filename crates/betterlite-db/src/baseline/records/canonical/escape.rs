pub(super) fn write(out: &mut String, text: &str) {
    // json.dumps(ensure_ascii=True): short escapes for the named controls, lowercase \uXXXX for the rest of the control range, DEL and every non-ASCII code point.
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ' '..='~' => out.push(character),
            other => write_escaped(out, other),
        }
    }
    out.push('"');
}

fn write_escaped(out: &mut String, character: char) {
    let mut units = [0_u16; 2];
    for unit in character.encode_utf16(&mut units) {
        out.push_str("\\u");
        for shift in [12_u32, 8, 4, 0] {
            let digit = (*unit >> shift) & 0xf;
            out.push(char::from_digit(u32::from(digit), 16).unwrap_or('0'));
        }
    }
}
