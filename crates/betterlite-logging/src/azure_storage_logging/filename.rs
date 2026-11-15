use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};

const RESPONSES_PREFIX: &str = "resp_";
const DOT_OR_EMPTY: [&str; 3] = ["", ".", ".."];
// the characters a URL path must escape; a / stays a separator, as in LiteLLM's f-string URL and the SDK's quote
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
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

// a resp_ base64 id or an empty, . or .. segment gets the slash and padding rewrite, and .json is always added
#[must_use]
pub fn adls_safe_file_name(payload_id: Option<&str>, ids: &dyn IdSource) -> String {
    let Some(name) = payload_id else {
        return format!("{}.json", ids.next_id());
    };
    // Python's unquote leaves a stray percent sign alone and replaces invalid UTF-8, as decode_utf8_lossy does
    let unsafe_segments = name.starts_with(RESPONSES_PREFIX)
        || percent_decode_str(name)
            .decode_utf8_lossy()
            .split('/')
            .any(|segment| DOT_OR_EMPTY.contains(&segment));
    if !unsafe_segments {
        return format!("{name}.json");
    }
    let sanitized = name.replace('/', "_").replace('=', "");
    format!("{sanitized}.json")
}

#[must_use]
pub fn storage_url(endpoint: &str, segments: &[&str]) -> String {
    let mut url = endpoint.to_owned();
    for segment in segments {
        url.push('/');
        url.extend(utf8_percent_encode(segment, PATH_SEGMENT));
    }
    url
}
