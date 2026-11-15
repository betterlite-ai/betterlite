use crate::videos::config::{
    DEFAULT_API_BASE, VideoCredentials, complete_url, validate_environment,
};

use super::support::{credentials, exposed, secret_headers};

type Sources = [Option<&'static str>; 5];

const CHAIN: [(Sources, &str); 9] = [
    (
        [
            Some("explicit"),
            Some("params"),
            Some("global"),
            Some("openai"),
            Some("environment"),
        ],
        "Bearer explicit",
    ),
    (
        [
            None,
            Some("params"),
            Some("global"),
            Some("openai"),
            Some("environment"),
        ],
        "Bearer params",
    ),
    (
        [
            Some(""),
            Some("params"),
            Some("global"),
            Some("openai"),
            Some("environment"),
        ],
        "Bearer params",
    ),
    (
        [
            None,
            None,
            Some("global"),
            Some("openai"),
            Some("environment"),
        ],
        "Bearer global",
    ),
    (
        [None, None, None, Some("openai"), Some("environment")],
        "Bearer openai",
    ),
    (
        [None, None, None, None, Some("environment")],
        "Bearer environment",
    ),
    (
        [Some(""), Some(""), Some(""), Some(""), Some("")],
        "Bearer None",
    ),
    ([Some(""), None, None, None, None], "Bearer None"),
    ([Some(" "), None, None, None, None], "Bearer  "),
];

fn bearer(configured: &VideoCredentials) -> String {
    let base = secret_headers(&[("Authorization", "Bearer old"), ("X-Trace", "trace-1")]);
    let updated = validate_environment(base, configured);
    assert_eq!(exposed(&updated, "X-Trace"), Some("trace-1"));
    exposed(&updated, "Authorization")
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn auth_chain_case() {
    for (sources, expected) in CHAIN {
        let [api, params, global, openai, environment] = sources;
        let configured = credentials(api, params, global, openai, environment);
        assert_eq!(bearer(&configured), expected);
    }
    assert_eq!(complete_url(None), format!("{DEFAULT_API_BASE}/videos"));
    assert_eq!(
        complete_url(Some("https://mock.example/v1///")),
        "https://mock.example/v1/videos"
    );
    assert_eq!(complete_url(Some("")), "/videos");
}
