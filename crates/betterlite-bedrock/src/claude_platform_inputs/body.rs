use std::collections::BTreeSet;

use betterlite_config::sections::claude_platform_inputs::{PlatformValue, UnsupportedParams};

use super::{DEFAULT_UNSUPPORTED, NON_REQUEST_PARAMS};
use crate::model_routes::ParamMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformWarning {
    DroppedUnsupported { params: Vec<String> },
    IgnoredOverride { type_name: &'static str },
}

impl PlatformWarning {
    pub const SUBJECT: &'static str = "bedrock/claude_platform";

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::DroppedUnsupported { params } => format!(
                "{subject}: dropping unsupported Messages API param(s) {params} from the request body; the Claude Platform on AWS endpoint rejects unknown fields. The request will proceed without them.",
                subject = Self::SUBJECT,
                params = rendered_params(params),
            ),
            Self::IgnoredOverride { type_name } => format!(
                "{subject}: ignoring claude_platform_unsupported_params of type {type_name}; expected a list of param names. Using the default unsupported-param set.",
                subject = Self::SUBJECT,
            ),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnsupportedResolution {
    pub names: Option<BTreeSet<String>>,
    pub warning: Option<PlatformWarning>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilteredBody {
    pub body: ParamMap,
    pub warning: Option<PlatformWarning>,
}

pub fn resolve_unsupported_override(
    deployment: Option<&UnsupportedParams>,
    optional: Option<&UnsupportedParams>,
    log_invalid: bool,
) -> UnsupportedResolution {
    let Some(raw) = non_null(optional).or_else(|| non_null(deployment)) else {
        return UnsupportedResolution::default();
    };
    let resolution = match raw {
        UnsupportedParams::List(items)
        | UnsupportedParams::Set(items)
        | UnsupportedParams::FrozenSet(items)
        | UnsupportedParams::Tuple(items) => UnsupportedResolution {
            names: Some(items.iter().map(rendered_item).collect()),
            warning: None,
        },
        UnsupportedParams::Other(value) => UnsupportedResolution {
            names: None,
            warning: log_invalid.then(|| PlatformWarning::IgnoredOverride {
                type_name: value_type_name(value),
            }),
        },
    };
    if let Some(warning) = &resolution.warning {
        tracing::warn!("{}", warning.message());
    }
    resolution
}

// LiteLLM reads a null override as absent, so an explicit null never warns
const fn non_null(value: Option<&UnsupportedParams>) -> Option<&UnsupportedParams> {
    match value {
        Some(UnsupportedParams::Other(PlatformValue::Null)) => None,
        _ => value,
    }
}

pub fn filter_request_body(
    params: &ParamMap,
    unsupported_override: Option<&BTreeSet<String>>,
    log_dropped: bool,
) -> FilteredBody {
    let default: BTreeSet<String> = DEFAULT_UNSUPPORTED
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let unsupported = unsupported_override.unwrap_or(&default);
    let dropped: Vec<String> = params
        .keys()
        .filter(|key| unsupported.contains(*key))
        .cloned()
        .collect();
    let warning = (log_dropped && !dropped.is_empty())
        .then_some(PlatformWarning::DroppedUnsupported { params: dropped });
    if let Some(entry) = &warning {
        tracing::warn!("{}", entry.message());
    }
    let body = params
        .iter()
        .filter(|(key, _)| {
            !NON_REQUEST_PARAMS.contains(&key.as_str())
                && !unsupported.contains(key.as_str())
                && !key.starts_with("aws_")
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    FilteredBody { body, warning }
}

fn rendered_params(params: &[String]) -> String {
    match params {
        [] => "()".to_owned(),
        names => format!("(\"{}\")", names.join("\", \"")),
    }
}

fn value_type_name(value: &PlatformValue) -> &'static str {
    match value {
        PlatformValue::Null => "NoneType",
        PlatformValue::Boolean(_) => "bool",
        PlatformValue::Number(number) => {
            if number.is_f64() {
                "float"
            } else {
                "int"
            }
        }
        PlatformValue::Text(_) => "str",
        PlatformValue::List(_) => "list",
        PlatformValue::Mapping(_) => "dict",
    }
}

fn rendered_item(value: &PlatformValue) -> String {
    match value {
        PlatformValue::Null => "None".to_owned(),
        PlatformValue::Boolean(true) => "True".to_owned(),
        PlatformValue::Boolean(false) => "False".to_owned(),
        PlatformValue::Number(number) => number.to_string(),
        PlatformValue::Text(text) => text.clone(),
        PlatformValue::List(items) => {
            let rendered: Vec<String> = items.iter().map(rendered_item).collect();
            format!("[{}]", rendered.join(", "))
        }
        PlatformValue::Mapping(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(key, item)| format!("{key}: {}", rendered_item(item)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}
