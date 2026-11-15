use url::Url;

const OPENAI_HOSTNAMES: [&str; 1] = ["api.openai.com"];
const AZURE_OPENAI_HOSTNAMES: [&str; 2] = ["openai.azure.com", "cognitiveservices.azure.com"];
const AZURE_OPENAI_PATH_MARKERS: [&str; 2] = ["/openai/", "/v1/"];

mod tests;

pub(crate) struct ParsedRoute {
    pub(crate) hostname: Option<String>,
    pub(crate) path: String,
}

// urlsplit strips leading C0 controls and spaces, deletes tab, CR and LF, and keeps a backslash or an inner space as data rather than a separator
fn clean_route(route: &str) -> String {
    route
        .trim_start_matches(|character: char| character <= ' ')
        .replace(['\t', '\n', '\r'], "")
        .replace('\\', "%5C")
        .replace(' ', "%20")
}

// urlsplit reads a netloc only between "//" and the next "/", "?" or "#"; three leading slashes give an empty netloc and no host
fn has_netloc(rest: &str) -> bool {
    rest.strip_prefix("//")
        .and_then(|authority| authority.chars().next())
        .is_some_and(|first| !matches!(first, '/' | '?' | '#'))
}

// A URL only has a host for urlparse when its scheme is followed by "//"; the url crate also accepts http:host and http:/host
fn netloc_after_scheme(cleaned: &str, url: &Url) -> bool {
    cleaned
        .get(url.scheme().len().saturating_add(1)..)
        .is_some_and(has_netloc)
}

// A scheme-relative route is read as http so the url crate accepts it
fn scheme_relative(cleaned: &str) -> Option<Url> {
    if has_netloc(cleaned) {
        Url::parse(&format!("http:{cleaned}")).ok()
    } else {
        None
    }
}

pub(crate) fn parse_route(route: &str) -> ParsedRoute {
    let cleaned = clean_route(route);
    let parsed = match Url::parse(&cleaned) {
        Ok(url) => netloc_after_scheme(&cleaned, &url).then_some(url),
        Err(_failure) => scheme_relative(&cleaned),
    };
    match parsed {
        Some(url) => ParsedRoute {
            hostname: url.host_str().map(str::to_owned),
            path: url.path().to_owned(),
        },
        None => ParsedRoute {
            hostname: None,
            path: String::new(),
        },
    }
}

// The source compares hostnames as given; urlparse has already lowercased the one it reads off a URL
fn hostname_matches(hostname: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|suffix| {
        hostname == *suffix
            || hostname
                .strip_suffix(*suffix)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

#[must_use]
pub fn is_openai_compatible_host(hostname: Option<&str>) -> bool {
    let Some(hostname) = hostname else {
        return false;
    };
    if hostname.is_empty() {
        return false;
    }
    hostname_matches(hostname, &OPENAI_HOSTNAMES)
        || hostname_matches(hostname, &AZURE_OPENAI_HOSTNAMES)
}

#[must_use]
pub fn is_openai_compatible_url(route: &str) -> bool {
    let parsed = parse_route(route);
    let Some(hostname) = parsed.hostname.as_deref() else {
        return false;
    };
    if hostname_matches(hostname, &OPENAI_HOSTNAMES) {
        return true;
    }
    hostname_matches(hostname, &AZURE_OPENAI_HOSTNAMES)
        && AZURE_OPENAI_PATH_MARKERS
            .iter()
            .any(|marker| parsed.path.contains(*marker))
}
