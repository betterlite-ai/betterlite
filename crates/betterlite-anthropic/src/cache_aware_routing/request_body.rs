use url::Url;

// urlparse().path keeps everything before the query and fragment, whether or not the URL names a scheme.
pub(super) fn url_path(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        let end = url.find(['?', '#']).unwrap_or(url.len());
        return url.get(..end).unwrap_or_default().to_owned();
    };
    parsed.path().to_owned()
}

mod tests;
