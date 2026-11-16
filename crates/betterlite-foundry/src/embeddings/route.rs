use crate::credentials::host::is_foundry_base;

const MODELS_SUFFIX: &str = "/models";

struct BaseParts<'base> {
    scheme: &'base str,
    netloc: &'base str,
    path: &'base str,
    query: &'base str,
    fragment: &'base str,
}

// LiteLLM routes a Foundry model-inference base through /models, an unchanged path keeps the original string
#[must_use]
pub fn models_route_base(api_base: Option<&str>) -> Option<String> {
    let base = api_base?;
    if !is_foundry_base(base) {
        return Some(base.to_owned());
    }
    let parts = split_base(base);
    let path = parts.path.trim_end_matches('/');
    if path.ends_with(MODELS_SUFFIX) {
        return Some(base.to_owned());
    }
    Some(rebuilt(&parts, path))
}

// urlsplit/urlunsplit keep the netloc verbatim, lowercase the scheme and drop an empty query or fragment
fn split_base(base: &str) -> BaseParts<'_> {
    let (before_fragment, fragment) = base.split_once('#').unwrap_or((base, ""));
    let (scheme, rest) = scheme_prefix(before_fragment);
    let (netloc, after_netloc) = rest.strip_prefix("//").map_or(("", rest), |authority| {
        authority
            .find(['/', '?'])
            .map_or((authority, ""), |delimiter| authority.split_at(delimiter))
    });
    let (path, query) = after_netloc.split_once('?').unwrap_or((after_netloc, ""));
    BaseParts {
        scheme,
        netloc,
        path,
        query,
        fragment,
    }
}

fn scheme_prefix(base: &str) -> (&str, &str) {
    let Some(colon) = base.find(':') else {
        return ("", base);
    };
    let scheme = &base[..colon];
    let named = scheme
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        });
    if named {
        (scheme, &base[colon + 1..])
    } else {
        ("", base)
    }
}

fn rebuilt(parts: &BaseParts<'_>, path: &str) -> String {
    let mut url = String::new();
    if !parts.scheme.is_empty() {
        url.push_str(&parts.scheme.to_ascii_lowercase());
        url.push(':');
    }
    url.push_str("//");
    url.push_str(parts.netloc);
    url.push_str(path);
    url.push_str(MODELS_SUFFIX);
    if !parts.query.is_empty() {
        url.push('?');
        url.push_str(parts.query);
    }
    if !parts.fragment.is_empty() {
        url.push('#');
        url.push_str(parts.fragment);
    }
    url
}
