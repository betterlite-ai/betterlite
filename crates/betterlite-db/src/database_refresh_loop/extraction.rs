use percent_encoding::percent_decode_str;

use crate::database_url_policy::url_codec::urlsplit;

// The URL is split before any decoding, so a percent-encoded '?' in the token cannot start the query.
#[must_use]
pub fn extract_refresh_password(db_url: &str) -> Option<String> {
    let parsed = urlsplit(db_url).ok()?;
    let (userinfo, _hostport) = parsed.netloc.rsplit_once('@')?;
    let (_user, password) = userinfo.split_once(':')?;
    if password.is_empty() {
        return None;
    }
    // urllib.parse.unquote decodes once, leaves a bad escape as a literal '%' and replaces invalid UTF-8
    Some(
        percent_decode_str(password)
            .decode_utf8_lossy()
            .into_owned(),
    )
}
