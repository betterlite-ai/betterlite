use super::AUTH_ENDPOINT_SUFFIX;

#[derive(Debug, Clone, Copy)]
pub(super) enum Transform {
    AuthEndpoint,
    ApiBase,
    PemText,
}

impl Transform {
    fn apply(self, value: &str) -> String {
        match self {
            Self::AuthEndpoint => append_suffix(value, AUTH_ENDPOINT_SUFFIX),
            Self::ApiBase => append_suffix(value, "/v2"),
            Self::PemText => value.replace("\\n", "\n"),
        }
    }
}

fn append_suffix(value: &str, suffix: &str) -> String {
    let trimmed = value.trim_end_matches('/');
    if value.ends_with(suffix) {
        return trimmed.to_owned();
    }
    format!("{trimmed}{suffix}")
}

pub(super) type CredentialSpec = (
    &'static str,
    Option<&'static [&'static str]>,
    Option<Transform>,
);

pub(super) const CREDENTIAL_VALUES: [CredentialSpec; 9] = [
    ("client_id", Some(&["clientid"]), None),
    ("client_secret", Some(&["clientsecret"]), None),
    ("auth_url", Some(&["url"]), Some(Transform::AuthEndpoint)),
    (
        "base_url",
        Some(&["serviceurls", "AI_API_URL"]),
        Some(Transform::ApiBase),
    ),
    (
        "cert_url",
        Some(&["certurl"]),
        Some(Transform::AuthEndpoint),
    ),
    ("cert_file_path", None, None),
    ("key_file_path", None, None),
    ("cert_str", Some(&["certificate"]), Some(Transform::PemText)),
    ("key_str", Some(&["key"]), Some(Transform::PemText)),
];

pub(super) fn transformed(transform: Option<Transform>, value: &str) -> String {
    if let Some(kind) = transform {
        return kind.apply(value);
    }
    value.to_owned()
}
