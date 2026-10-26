use betterlite_config::loader::EnvironmentLookup;
use serde::Deserialize;
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory as _};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub source: String,
    pub service: String,
    pub hostname: String,
    pub env: String,
    pub pod_name: String,
    pub version: String,
    pub base_url: Option<String>,
    pub use_tracer: bool,
    pub use_profiler: bool,
}

impl Environment {
    pub fn from_lookup(lookup: &dyn EnvironmentLookup) -> Self {
        let text =
            |name: &str, default: &str| lookup.lookup(name).unwrap_or_else(|| default.to_owned());
        // LiteLLM's str_to_bool takes only "true", so "1" or "yes" leave the flag off
        let flag = |name: &str| text(name, "").trim().eq_ignore_ascii_case("true");
        Self {
            source: text("DD_SOURCE", "litellm"),
            service: text("DD_SERVICE", "litellm-server"),
            hostname: text("HOSTNAME", ""),
            env: text("DD_ENV", "unknown"),
            pod_name: text("POD_NAME", "unknown"),
            version: text("DD_VERSION", "unknown"),
            base_url: lookup.lookup("DD_BASE_URL"),
            use_tracer: flag("USE_DDTRACE"),
            use_profiler: flag("USE_DDPROFILER"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TeamMetadata {
    pub user_api_key_team_alias: Option<String>,
    pub team_alias: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestTagValue {
    Null,
    Bool(bool),
    Number(String),
    Text(String),
}

impl RequestTagValue {
    fn display_value(&self) -> &str {
        match self {
            Self::Null => "none",
            Self::Bool(true) => "true",
            Self::Bool(false) => "false",
            Self::Number(value) | Self::Text(value) => value,
        }
    }
}

#[must_use]
pub fn normalize_value(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for character in value.to_lowercase().chars() {
        // Python's isalnum accepts letters and numbers, unlike Rust's broader Alphabetic property
        let alphanumeric = matches!(
            character.general_category_group(),
            GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
        );
        let kept = if alphanumeric || "_-:./".contains(character) {
            character
        } else {
            '_'
        };
        if kept != '_' || !normalized.ends_with('_') {
            normalized.push(kept);
        }
    }
    normalized.trim_matches('_').to_owned()
}

#[must_use]
pub fn tags(
    environment: &Environment,
    request_tags: &[RequestTagValue],
    team: &TeamMetadata,
) -> Vec<String> {
    let mut tags = vec![
        format!("env:{}", environment.env),
        format!("service:{}", environment.service),
        format!("version:{}", environment.version),
        format!("HOSTNAME:{}", environment.hostname),
        format!("POD_NAME:{}", environment.pod_name),
    ];
    for tag in request_tags {
        tags.push(format!(
            "request_tag:{}",
            normalize_value(tag.display_value())
        ));
    }
    // LiteLLM chains these with `or`, so an empty value falls through to the next one
    let candidates = [
        team.user_api_key_team_alias.as_deref(),
        team.team_alias.as_deref(),
        team.user_api_key_team_id.as_deref(),
        team.team_id.as_deref(),
    ];
    if let Some(name) = candidates.iter().flatten().find(|name| !name.is_empty()) {
        tags.push(format!("team:{}", normalize_value(name)));
    }
    tags
}

mod tests;
