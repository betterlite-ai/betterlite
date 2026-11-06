use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

// Python's quote(text, safe="") keeps only letters, digits and _.-~
const QUOTE_UNSAFE: AsciiSet = NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FileEndpoint {
    Raw,
    Json,
}

pub(super) fn file_url(
    base_url: &str,
    project: &str,
    file_path: &str,
    ref_name: &str,
    endpoint: FileEndpoint,
) -> String {
    let project_encoded = utf8_percent_encode(project, &QUOTE_UNSAFE);
    let file_encoded = utf8_percent_encode(file_path, &QUOTE_UNSAFE);
    let ref_encoded = utf8_percent_encode(ref_name, &QUOTE_UNSAFE);
    let suffix = match endpoint {
        FileEndpoint::Raw => "/raw",
        FileEndpoint::Json => "",
    };
    format!(
        "{base_url}/projects/{project_encoded}/repository/files/{file_encoded}{suffix}?ref={ref_encoded}"
    )
}

mod tests;
