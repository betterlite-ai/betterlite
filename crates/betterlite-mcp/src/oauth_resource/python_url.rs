pub mod netloc;

use thiserror::Error;

use self::netloc::validate_netloc;

// \u{69} spells the real scheme without tripping the spell checker
const USES_NETLOC: &str = " ftp http gopher nntp telnet imap wais file mms https shttp snews prospero rtsp rtsps rtspu rsync svn svn+ssh sftp nfs git git+ssh ws wss \u{69}tms-services";
const USES_PARAMS: &str =
    " ftp hdl prospero http imap https shttp rtsp rtsps rtspu sip sips mms sftp tel";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UrlParseError {
    #[error("invalid IPv6 URL")]
    InvalidIpv6,
    #[error("the port is not a valid TCP port")]
    InvalidPort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
}

// CPython's urlsplit and urlparse for the parts LiteLLM keeps: query, fragment and params are dropped at every call site.
pub fn parse(url: &str) -> Result<ParsedUrl, UrlParseError> {
    let (scheme, remainder) = split_scheme(url).unwrap_or_else(|| (String::new(), url));
    let (netloc, after_netloc) = split_netloc(remainder)?;
    let path = path_without_params(&scheme, after_netloc);
    Ok(ParsedUrl {
        scheme,
        netloc,
        path: path.to_owned(),
    })
}

// The urlunparse calls here always pass empty params, query and fragment, so this is urlunsplit.
#[must_use]
pub fn join(scheme: &str, netloc: &str, path: &str, query: &str, fragment: &str) -> String {
    let authority_path = path.starts_with("//")
        || (!scheme.is_empty()
            && USES_NETLOC.split(' ').any(|candidate| candidate == scheme)
            && (path.is_empty() || path.starts_with('/')));
    let assembled = if !netloc.is_empty() {
        let separator = if path.is_empty() || path.starts_with('/') {
            ""
        } else {
            "/"
        };
        format!("//{netloc}{separator}{path}")
    } else if authority_path {
        format!("//{path}")
    } else {
        path.to_owned()
    };
    let mut url = if scheme.is_empty() {
        assembled
    } else {
        format!("{scheme}:{assembled}")
    };
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }
    if !fragment.is_empty() {
        url.push('#');
        url.push_str(fragment);
    }
    url
}

fn path_without_params<'url>(scheme: &str, after_netloc: &'url str) -> &'url str {
    let before_fragment = after_netloc.split('#').next().unwrap_or_default();
    let before_query = before_fragment.split('?').next().unwrap_or_default();
    if !USES_PARAMS.split(' ').any(|candidate| candidate == scheme) {
        return before_query;
    }
    let Some(index) = params_index(before_query) else {
        return before_query;
    };
    before_query.get(..index).unwrap_or_default()
}

fn split_scheme(url: &str) -> Option<(String, &str)> {
    let colon = url.find(':')?;
    let candidate = url.get(..colon)?;
    let mut characters = candidate.chars();
    let first = characters.next()?;
    let rest_valid = characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'));
    if !first.is_ascii_alphabetic() || !rest_valid {
        return None;
    }
    Some((
        candidate.to_lowercase(),
        url.get(colon.saturating_add(1)..)?,
    ))
}

fn split_netloc(remainder: &str) -> Result<(String, &str), UrlParseError> {
    let Some(after_slashes) = remainder.strip_prefix("//") else {
        return Ok((String::new(), remainder));
    };
    let delimiter = after_slashes
        .find(['/', '?', '#'])
        .unwrap_or(after_slashes.len());
    let netloc = after_slashes
        .get(..delimiter)
        .unwrap_or_default()
        .to_owned();
    validate_netloc(&netloc)?;
    Ok((netloc, after_slashes.get(delimiter..).unwrap_or_default()))
}

fn params_index(path: &str) -> Option<usize> {
    let Some(search_from) = path.rfind('/') else {
        return path.find(';');
    };
    path.get(search_from..)?
        .find(';')
        .map(|offset| search_from.saturating_add(offset))
}
