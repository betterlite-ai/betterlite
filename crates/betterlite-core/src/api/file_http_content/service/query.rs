use url::Url;

use super::super::config::QueryParams;

// httpx merges QueryParams(extract_query_params(url)).merge(prepared): a shared key keeps the URL's position and takes the prepared value
pub(super) fn streaming_url(url: &Url, params: &QueryParams) -> Url {
    if params.is_empty() {
        return url.clone();
    }
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (key, value) in url.query_pairs() {
        let name = key.into_owned();
        match params.get(&name) {
            Some(replacement) => {
                if !pairs.iter().any(|(present, _)| present == &name) {
                    pairs.push((name, replacement.clone()));
                }
            }
            None => pairs.push((name, value.into_owned())),
        }
    }
    for (name, value) in params {
        if !pairs.iter().any(|(present, _)| present == name) {
            pairs.push((name.clone(), value.clone()));
        }
    }
    let mut merged = url.clone();
    merged.query_pairs_mut().clear().extend_pairs(&pairs);
    merged
}
