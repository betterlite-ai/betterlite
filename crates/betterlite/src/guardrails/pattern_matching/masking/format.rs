#[must_use]
pub(super) fn string_padding(format: &str) -> String {
    let mut output = String::new();
    let mut characters = format.chars().peekable();
    let mut field = false;
    while let Some(character) = characters.next() {
        output.push(character);
        match character {
            '{' if !field && characters.next_if_eq(&'{').is_some() => output.push('{'),
            '{' => field = true,
            '}' => field = false,
            ':' if field => match (characters.peek().copied(), characters.clone().nth(1)) {
                (Some('0'), next)
                    if !next.is_some_and(|value| matches!(value, '<' | '>' | '^' | '=')) =>
                {
                    output.push_str("0<");
                    characters.next();
                }
                (Some(alignment @ ('<' | '>' | '^')), Some('0')) => {
                    output.push('0');
                    output.push(alignment);
                    characters.next();
                    characters.next();
                }
                _ => {}
            },
            _ => {}
        }
    }
    output
}
