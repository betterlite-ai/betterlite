mod tests;

const A2A_INVOKE_SUFFIX: &str = "/message/send";
const A2A_TRANSPORT_PREFIXES: &[&str] = &["/v1/a2a/", "/a2a/"];

pub(super) fn classify_a2a_route(path: &str) -> Option<&'static str> {
    let matches_prefix = A2A_TRANSPORT_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix));
    (path.ends_with(A2A_INVOKE_SUFFIX) && matches_prefix).then_some("/a2a")
}
