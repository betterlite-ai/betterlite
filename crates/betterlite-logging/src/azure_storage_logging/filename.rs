use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

const RESPONSES_PREFIX: &str = "resp_";
const DOT_OR_EMPTY: [&str; 3] = ["", ".", ".."];
// exactly the characters Python's urllib leaves in a path segment; an ordinary id passes through unchanged
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub trait IdSource: Send + Sync {
    fn next_id(&self) -> String;
}

pub struct RandomIds;

impl IdSource for RandomIds {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

// Python's unquote leaves a stray percent sign alone and replaces invalid UTF-8, which is decode_utf8_lossy
fn unquote(text: &str) -> String {
    percent_encoding::percent_decode_str(text)
        .decode_utf8_lossy()
        .into_owned()
}

// A Responses API id is base64 behind `resp_`, so its `=` padding and `/` are rewritten; every other id keeps its
// name unless a path segment is empty, `.` or `..`, which gets the same rewrite. The extension is always added.
#[must_use]
pub fn adls_safe_file_name(payload_id: Option<&str>, ids: &dyn IdSource) -> String {
    let Some(name) = payload_id else {
        return format!("{}.json", ids.next_id());
    };
    let unsafe_segments = name.starts_with(RESPONSES_PREFIX)
        || unquote(name)
            .split('/')
            .any(|segment| DOT_OR_EMPTY.contains(&segment));
    if !unsafe_segments {
        return format!("{name}.json");
    }
    let sanitized = name.replace('/', "_").replace('=', "");
    format!("{sanitized}.json")
}

#[must_use]
pub fn encode_path_segment(segment: &str) -> String {
    utf8_percent_encode(segment, PATH_SEGMENT).to_string()
}
