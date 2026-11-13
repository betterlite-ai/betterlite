mod tests;

// S3 answers an object-specific rejection with one of these XML codes, and only these are final
pub const TERMINAL_CODES: [&str; 7] = [
    "BadDigest",
    "EntityTooLarge",
    "InvalidArgument",
    "InvalidDigest",
    "InvalidRequest",
    "KeyTooLongError",
    "MalformedXML",
];

const CODE_OPEN: &str = "<Code>";
const CODE_CLOSE: &str = "</Code>";
const BODY_CODED_STATUSES: [u16; 2] = [400, 403];

#[must_use]
pub fn error_code(body: &str) -> Option<&str> {
    body.split(CODE_OPEN)
        .skip(1)
        .filter_map(|after_open| after_open.split_once(CODE_CLOSE).map(|(code, _)| code))
        .find(|code| !code.is_empty() && !code.contains('<'))
}

#[must_use]
pub fn is_terminal(status: u16, body: &str) -> bool {
    BODY_CODED_STATUSES.contains(&status)
        && error_code(body).is_some_and(|code| TERMINAL_CODES.contains(&code))
}
