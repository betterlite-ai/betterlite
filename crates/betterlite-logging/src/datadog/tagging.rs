use betterlite_config::loader::EnvironmentLookup;
use serde::Deserialize;

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

const fn is_combining_mark(character: char) -> bool {
    let combining = matches!(
        character,
        '\u{0300}'..='\u{036f}'
            | '\u{0483}'..='\u{0489}'
            | '\u{0591}'..='\u{05bd}'
            | '\u{05bf}'
            | '\u{05c1}'..='\u{05c2}'
            | '\u{05c4}'..='\u{05c5}'
            | '\u{0610}'..='\u{061a}'
            | '\u{064b}'..='\u{065f}'
            | '\u{0670}'
            | '\u{06d6}'..='\u{06dc}'
            | '\u{06df}'..='\u{06e4}'
            | '\u{06e7}'..='\u{06e8}'
            | '\u{06ea}'..='\u{06ed}'
            | '\u{0711}'
            | '\u{0730}'..='\u{074a}'
            | '\u{07a6}'..='\u{07b0}'
            | '\u{07eb}'..='\u{07f3}'
            | '\u{0816}'..='\u{0819}'
            | '\u{081b}'..='\u{0823}'
            | '\u{0825}'..='\u{0827}'
            | '\u{0829}'..='\u{082d}'
            | '\u{0859}'..='\u{085f}'
            | '\u{08d3}'..='\u{08ff}'
            | '\u{0900}'..='\u{0903}'
            | '\u{093a}'..='\u{093c}'
            | '\u{093e}'..='\u{094f}'
            | '\u{0951}'..='\u{0957}'
            | '\u{0962}'..='\u{0963}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{20d0}'..='\u{20ff}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{fe20}'..='\u{fe2f}'
            | '\u{e0100}'..='\u{e01ef}'
    );
    combining
}

#[must_use]
pub fn normalize_value(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for character in value.to_lowercase().chars() {
        let kept = if (character.is_alphanumeric() && !is_combining_mark(character))
            || "_-:./".contains(character)
        {
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
