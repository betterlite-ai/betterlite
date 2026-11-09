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
            Self::AuthEndpoint => {
                let trimmed = value.trim_end_matches('/');
                let suffix = if value.ends_with(AUTH_ENDPOINT_SUFFIX) {
                    ""
                } else {
                    AUTH_ENDPOINT_SUFFIX
                };
                format!("{trimmed}{suffix}")
            }
            Self::ApiBase => {
                let trimmed = value.trim_end_matches('/');
                let suffix = if value.ends_with("/v2") { "" } else { "/v2" };
                format!("{trimmed}{suffix}")
            }
            Self::PemText => value.replace("\\n", "\n"),
        }
    }
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
