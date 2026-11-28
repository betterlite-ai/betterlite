mod tests;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

// Python's quote_plus always leaves the RFC 3986 unreserved marks alone.
const FORM_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

// Starlette's RedirectResponse quotes the whole URL with safe=":/%#?=@[]!$&'()*+,;".
const LOCATION_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b':')
    .remove(b'/')
    .remove(b'%')
    .remove(b'#')
    .remove(b'?')
    .remove(b'=')
    .remove(b'@')
    .remove(b'[')
    .remove(b']')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';');

// urllib.parse.urlencode quotes each value with quote_plus, which spells a space "+".
#[must_use]
pub(super) fn redirect_location(redirect_uri: &str, code: &str, state: &str) -> String {
    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    let quoted_code = utf8_percent_encode(code, FORM_SAFE)
        .to_string()
        .replace("%20", "+");
    let quoted_state = utf8_percent_encode(state, FORM_SAFE)
        .to_string()
        .replace("%20", "+");
    let location = format!("{redirect_uri}{separator}code={quoted_code}&state={quoted_state}");
    utf8_percent_encode(&location, LOCATION_SAFE).to_string()
}
