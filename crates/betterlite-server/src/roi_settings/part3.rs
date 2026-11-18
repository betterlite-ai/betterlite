#[derive(Debug, Serialize)]
struct DetailBody {
    detail: Detail,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Detail {
    Text(String),
    Issues(Vec<Issue>),
}

#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: Vec<LocPart>,
    msg: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum LocPart {
    Field(String),
    Index(usize),
}

#[derive(Debug, Error)]
pub enum RoiSettingsError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("the caller is not allowed to use the ROI Calculator")]
    Forbidden(&'static str),
    #[error("Stored ROI Calculator settings are invalid.")]
    StoredSettings,
    #[error("Stored ROI Calculator report is invalid.")]
    StoredReport,
    #[error("GitHub API URL cannot be null.")]
    NullUrl,
    #[error("the request body is empty")]
    EmptyBody,
    #[error("the request body is not valid JSON")]
    InvalidJson,
    #[error("the request body does not decode")]
    Body(#[source] Box<serde_path_to_error::Error<serde_json::Error>>),
    #[error("the request does not match the declared types")]
    Request(Vec<Issue>),
    #[error("the settings do not validate")]
    Validation(#[source] SettingsError),
    #[error("the ROI settings request failed")]
    Internal(#[source] InternalError),
}

#[derive(Debug, Error)]
pub enum InternalError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
    #[error("no salt key is configured to encrypt secrets with")]
    Encryption,
    #[error("the database is not connected")]
    DbNotConnected,
}

impl IntoResponse for RoiSettingsError {
    #[expect(
        clippy::too_many_lines,
        reason = "one response mapping keeps every LiteLLM status and detail paired in one place"
    )]
    fn into_response(self) -> Response {
        let (status, detail) = match self {
            Self::Unauthenticated => return unauthenticated(),
            Self::Forbidden(denial) => (StatusCode::FORBIDDEN, Detail::Text(denial.to_owned())),
            Self::StoredSettings => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Detail::Text(INVALID_SETTINGS.to_owned()),
            ),
            Self::StoredReport => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Detail::Text(INVALID_REPORT.to_owned()),
            ),
            Self::NullUrl => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Detail::Text(NULL_URL.to_owned()),
            ),
            Self::EmptyBody => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Detail::Issues(vec![missing_body()]),
            ),
            Self::InvalidJson => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Detail::Issues(vec![invalid_json()]),
            ),
            Self::Body(source) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Detail::Issues(vec![body_issue(&source)]),
            ),
            Self::Request(issues) => (StatusCode::UNPROCESSABLE_ENTITY, Detail::Issues(issues)),
            Self::Validation(error) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Detail::Issues(vec![settings_issue(&error)]),
            ),
            Self::Internal(error) => {
                let (status, text) = match &error {
                    InternalError::DbNotConnected => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        CommonProxyErrors::DbNotConnectedError.as_str(),
                    ),
                    InternalError::Config(_)
                    | InternalError::Encode(_)
                    | InternalError::Encryption => {
                        tracing::error!(error = ?error, "the ROI settings request failed");
                        (StatusCode::INTERNAL_SERVER_ERROR, INTERNAL)
                    }
                };
                (status, Detail::Text(text.to_owned()))
            }
        };
        (status, Json(DetailBody { detail })).into_response()
    }
}

fn missing_body() -> Issue {
    Issue {
        kind: "missing",
        loc: vec![LocPart::Field("body".to_owned())],
        msg: "Field required".to_owned(),
    }
}

fn invalid_json() -> Issue {
    Issue {
        kind: "json_invalid",
        loc: vec![LocPart::Field("body".to_owned())],
        msg: "JSON decode error".to_owned(),
    }
}

fn body_issue(source: &serde_path_to_error::Error<serde_json::Error>) -> Issue {
    let message = source.inner().to_string();
    if let Some(field) = unknown_field(&message) {
        return Issue {
            kind: "extra_forbidden",
            loc: vec![LocPart::Field("body".to_owned()), LocPart::Field(field)],
            msg: "Extra inputs are not permitted".to_owned(),
        };
    }
    let mut loc = vec![LocPart::Field("body".to_owned())];
    for segment in source.path() {
        match segment {
            Segment::Seq { index } => loc.push(LocPart::Index(*index)),
            Segment::Map { key } => loc.push(LocPart::Field(key.clone())),
            Segment::Enum { variant } => loc.push(LocPart::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
    let kind = if loc.len() == 1 {
        if message.starts_with("invalid type: null") {
            "missing"
        } else {
            "model_attributes_type"
        }
    } else {
        "value_error"
    };
    Issue {
        kind,
        loc,
        msg: message,
    }
}

fn unknown_field(message: &str) -> Option<String> {
    let rest = message.strip_prefix("unknown field `")?;
    let (field, _) = rest.split_once('`')?;
    Some(field.to_owned())
}

fn settings_issue(error: &SettingsError) -> Issue {
    let (kind, field) = match error {
        SettingsError::NotFinite { field } => ("finite_number", *field),
        SettingsError::TooSmall { field, .. } => ("greater_than_equal", *field),
        SettingsError::TooLarge { field, .. } => ("less_than_equal", *field),
        SettingsError::Interval => ("value_error", "update_interval_minutes"),
        SettingsError::Repositories => ("value_error", "repos"),
        SettingsError::Prompt => ("value_error", "estimator_prompt"),
        SettingsError::Login | SettingsError::Identity => ("value_error", "identity_map"),
        SettingsError::MissingGitHubUrl
        | SettingsError::GitHubUrl
        | SettingsError::Ipv6Url
        | SettingsError::IpvFuture
        | SettingsError::BracketedIpv4
        | SettingsError::BracketedHost { .. }
        | SettingsError::NetlocNormalization { .. } => ("value_error", "github_api_url"),
    };
    Issue {
        kind,
        loc: vec![LocPart::Field(field.to_owned())],
        msg: error.to_string(),
    }
}

fn request_issues(patch: &ROISettingsUpdate) -> Vec<Issue> {
    let Err(error) = patch.clone().validate() else {
        return Vec::new();
    };
    let mut issue = settings_issue(&error);
    issue.loc.insert(0, LocPart::Field("body".to_owned()));
    vec![issue]
}
