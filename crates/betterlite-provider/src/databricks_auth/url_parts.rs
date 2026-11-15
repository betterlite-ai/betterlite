struct UrlParts<'value> {
    scheme: Option<&'value str>,
    authority: Option<&'value str>,
    path: &'value str,
}

fn split(url: &str) -> UrlParts<'_> {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let (rest, _) = url.split_at(end);
    if let Some((scheme, after)) = rest.split_once("://")
        && is_scheme(scheme)
    {
        let (authority, path) = split_authority(after);
        return UrlParts {
            scheme: Some(scheme),
            authority: Some(authority),
            path,
        };
    }
    if let Some(after) = rest.strip_prefix("//") {
        let (authority, path) = split_authority(after);
        return UrlParts {
            scheme: None,
            authority: Some(authority),
            path,
        };
    }
    UrlParts {
        scheme: None,
        authority: None,
        path: rest,
    }
}

fn split_authority(after: &str) -> (&str, &str) {
    after
        .find('/')
        .map_or((after, ""), |index| after.split_at(index))
}

fn is_scheme(text: &str) -> bool {
    let mut letters = text.chars();
    letters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && letters.all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '+' | '-' | '.'))
}

#[must_use]
pub fn path(url: &str) -> &str {
    // only the gateway branch reads a path, and it asks whether Python's urlsplit saw one
    split(url).path
}

// Python's urlunsplit((scheme, netloc, "", "", "")) keeps only the scheme and the authority
#[must_use]
pub fn workspace(url: &str) -> String {
    let parts = split(url);
    match (parts.scheme, parts.authority) {
        (Some(scheme), Some(authority)) => format!("{}://{authority}", scheme.to_ascii_lowercase()),
        (None, Some(authority)) => format!("//{authority}"),
        (_, None) => String::new(),
    }
}
