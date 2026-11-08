use std::iter::Peekable;
use std::str::Chars;

use crate::loader::EnvironmentLookup;

pub(super) fn expand_variables<L>(lookup: &L, text: &str) -> String
where
    L: EnvironmentLookup,
{
    let mut expanded = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            expanded.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('{') => {
                chars.next();
                expand_braced(&mut chars, lookup, &mut expanded);
            }
            Some(next) if next.is_ascii_alphanumeric() || next == '_' => {
                expand_plain(&mut chars, lookup, &mut expanded);
            }
            _ => expanded.push('$'),
        }
    }
    expanded
}

fn expand_braced<L>(chars: &mut Peekable<Chars<'_>>, lookup: &L, expanded: &mut String)
where
    L: EnvironmentLookup,
{
    let mut name = String::new();
    let mut closed = false;
    for inner in chars.by_ref() {
        if inner == '}' {
            closed = true;
            break;
        }
        name.push(inner);
    }
    match (closed, lookup.lookup(&name)) {
        (true, Some(value)) => expanded.push_str(&value),
        (true, None) => {
            expanded.push_str("${");
            expanded.push_str(&name);
            expanded.push('}');
        }
        (false, _) => {
            expanded.push_str("${");
            expanded.push_str(&name);
        }
    }
}

fn expand_plain<L>(chars: &mut Peekable<Chars<'_>>, lookup: &L, expanded: &mut String)
where
    L: EnvironmentLookup,
{
    let mut name = String::new();
    while let Some(candidate) = chars.peek().copied() {
        if !(candidate.is_ascii_alphanumeric() || candidate == '_') {
            break;
        }
        name.push(candidate);
        chars.next();
    }
    if let Some(value) = lookup.lookup(&name) {
        expanded.push_str(&value);
    } else {
        expanded.push('$');
        expanded.push_str(&name);
    }
}

pub(super) fn expand_home<L>(lookup: &L, text: &str) -> String
where
    L: EnvironmentLookup,
{
    let Some(rest) = text.strip_prefix('~') else {
        return text.to_owned();
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        return text.to_owned();
    }
    let Some(home) = home(lookup) else {
        return text.to_owned();
    };
    format!("{home}{rest}")
}

fn home<L>(lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    if let Some(home) = lookup.lookup("HOME").filter(|home| !home.is_empty()) {
        return Some(home);
    }
    let path = lookup.system_home()?;
    path.to_str().map(str::to_owned)
}
