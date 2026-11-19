const FIRST_CONTRIBUTION: &str = " made their first contribution";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleaseCounts {
    pub new_features: u64,
    pub bug_fixes: u64,
    pub other_updates: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bucket {
    NewFeatures,
    BugFixes,
    Other,
}

#[must_use]
pub fn count_release_bullets(body: &str) -> ReleaseCounts {
    let mut counts = ReleaseCounts::default();
    for line in release_lines(body) {
        match bucket_for(line) {
            Some(Bucket::NewFeatures) => {
                counts.new_features = counts.new_features.saturating_add(1);
            }
            Some(Bucket::BugFixes) => counts.bug_fixes = counts.bug_fixes.saturating_add(1),
            Some(Bucket::Other) => counts.other_updates = counts.other_updates.saturating_add(1),
            None => {}
        }
    }
    counts
}

fn bucket_for(line: &str) -> Option<Bucket> {
    let body = bullet_body(line)?;
    if is_first_contribution(body) {
        return None;
    }
    Some(prefix_bucket(body))
}

fn bullet_body(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('*')?;
    let body = rest.trim_start_matches(source_space);
    if body.len() == rest.len() || body.is_empty() {
        return None;
    }
    Some(body)
}

fn is_first_contribution(body: &str) -> bool {
    let Some(author) = body.strip_prefix('@') else {
        return false;
    };
    let token_len = author.find(source_space).unwrap_or(author.len());
    let Some((token, after)) = author.split_at_checked(token_len) else {
        return false;
    };
    if token.is_empty() {
        return false;
    }
    let Some(remaining) = after.strip_prefix(FIRST_CONTRIBUTION) else {
        return false;
    };
    remaining
        .chars()
        .next()
        .is_none_or(|next| !source_word(next))
}

fn prefix_bucket(body: &str) -> Bucket {
    let letters = body.bytes().take_while(u8::is_ascii_alphabetic).count();
    let Some((prefix, after_prefix)) = body.split_at_checked(letters) else {
        return Bucket::Other;
    };
    if prefix.is_empty() {
        return Bucket::Other;
    }
    let Some(after_scope) = skip_scope(after_prefix) else {
        return Bucket::Other;
    };
    let after_bang = after_scope.strip_prefix('!').unwrap_or(after_scope);
    let Some(after_colon) = after_bang.strip_prefix(':') else {
        return Bucket::Other;
    };
    let Some(after_space) = strip_one_space(after_colon) else {
        return Bucket::Other;
    };
    match after_space.chars().next() {
        Some(next) if !source_space(next) => bucket_named(prefix),
        Some(_) | None => Bucket::Other,
    }
}

fn skip_scope(after_prefix: &str) -> Option<&str> {
    let Some(scoped) = after_prefix.strip_prefix('(') else {
        return Some(after_prefix);
    };
    let close = scoped.find(')')?;
    let (_, remaining) = scoped.split_at_checked(close)?;
    remaining.strip_prefix(')')
}

const fn bucket_named(prefix: &str) -> Bucket {
    if prefix.eq_ignore_ascii_case("feat") {
        Bucket::NewFeatures
    } else if prefix.eq_ignore_ascii_case("fix") {
        Bucket::BugFixes
    } else {
        Bucket::Other
    }
}

fn strip_one_space(text: &str) -> Option<&str> {
    let mut characters = text.chars();
    let first = characters.next()?;
    let remainder = characters.as_str();
    source_space(first).then_some(remainder)
}

const fn source_space(character: char) -> bool {
    if matches!(character, '\u{1c}'..='\u{1f}') {
        return true;
    }
    character.is_whitespace()
}

fn source_word(character: char) -> bool {
    if character == '_' {
        return true;
    }
    character.is_alphanumeric()
}

const fn line_boundary(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r'
            | '\u{b}'
            | '\u{c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

fn release_lines(body: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut after_carriage_return = false;
    for (index, character) in body.char_indices() {
        if after_carriage_return && character == '\n' {
            after_carriage_return = false;
            start = index.saturating_add(character.len_utf8());
            continue;
        }
        after_carriage_return = character == '\r';
        if !line_boundary(character) {
            continue;
        }
        if let Some(line) = body.get(start..index) {
            lines.push(line);
        }
        start = index.saturating_add(character.len_utf8());
    }
    let tail = body.get(start..).unwrap_or_default();
    if !tail.is_empty() {
        lines.push(tail);
    }
    lines
}
