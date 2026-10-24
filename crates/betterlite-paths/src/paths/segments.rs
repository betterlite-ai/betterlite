use crate::error::PathError;
use crate::paths::{PathLimits, Segment};

pub fn parse(limits: &PathLimits, path: &str) -> Result<Vec<Segment>, PathError> {
    limits.check_length(path)?;
    let mut segments = Vec::new();
    let mut selectors = 0_usize;
    let mut rest = path;
    while !rest.is_empty() {
        let (segment, after) = scan(rest);
        rest = after;
        let Some(found) = segment else {
            continue;
        };
        if !matches!(found, Segment::Field(_)) {
            selectors = selectors.saturating_add(1);
        }
        if selectors > limits.max_selectors {
            return Err(PathError::TooManySelectors {
                limit: limits.max_selectors,
            });
        }
        segments.push(found);
        limits.check_depth(segments.len())?;
    }
    Ok(segments)
}

// one step of LiteLLM's findall over [^\.\[]+|\[[^\]]*\]: a dot or an unclosed bracket is skipped
fn scan(rest: &str) -> (Option<Segment>, &str) {
    if let Some(after) = rest.strip_prefix('.') {
        return (None, after);
    }
    if let Some(inner) = rest.strip_prefix('[') {
        return inner
            .split_once(']')
            .map_or((None, inner), |(content, after)| {
                (Some(bracket(content)), after)
            });
    }
    let (name, after) = rest
        .find(['.', '['])
        .and_then(|end| rest.split_at_checked(end))
        .unwrap_or((rest, ""));
    (Some(Segment::Field(name.to_owned())), after)
}

fn bracket(content: &str) -> Segment {
    if content == "*" {
        return Segment::Wildcard;
    }
    python_index(content).map_or(Segment::Unmatched, Segment::Index)
}

fn python_index(content: &str) -> Option<usize> {
    let trimmed = content.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
    });
    let (negative, unsigned) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |digits| (true, digits),
    );
    let grouped = unsigned
        .split('_')
        .all(|group| !group.is_empty() && group.bytes().all(|byte| byte.is_ascii_digit()));
    let digits = unsigned.replace('_', "");
    // Python's int() refuses strings of more than 4300 digits
    if !grouped || digits.len() > 4300 {
        return None;
    }
    let index = digits.parse::<usize>().ok()?;
    if negative && index != 0 {
        return None;
    }
    Some(index)
}

mod tests;
