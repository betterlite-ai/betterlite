use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Error, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde::de::IgnoredAny;

// the index is built once and then only read, so the whole route config stays in memory under one byte bound
pub const ROUTE_INDEX_MAX_BYTES: usize = 0x0010_0000;
// LiteLLMRouterEncoder's score threshold default, used by every route that sets no threshold of its own
pub const DEFAULT_SCORE_THRESHOLD: f64 = 0.3;
// one byte past the bound, so a file at the limit reads whole while a longer one still proves TooLarge
const READ_LIMIT_BYTES: u64 = 0x0010_0001;

#[derive(Debug, thiserror::Error)]
pub enum RouteIndexError {
    #[error("the route index input is over the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error("the route config at {} could not be read", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: Error,
    },
    #[error("the route config at {} must end in .json, .yaml or .yml", path.display())]
    UnsupportedExtension { path: PathBuf },
    #[error("the route config is not valid json: {0}")]
    Json(#[source] serde_json::Error),
    #[error("the route config is not valid yaml: {0}")]
    Yaml(#[source] serde_saphyr::Error),
    #[error("no auto router config was provided")]
    MissingConfig,
    #[error("route {name} has a score threshold that is not finite")]
    NonFiniteThreshold { name: String },
}

impl RouteIndexError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "too_large",
            Self::Read { .. } => "read_failed",
            Self::UnsupportedExtension { .. } => "unsupported_extension",
            Self::Json(_) => "invalid_json",
            Self::Yaml(_) => "invalid_yaml",
            Self::MissingConfig => "missing_config",
            Self::NonFiniteThreshold { .. } => "non_finite_threshold",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteGroup {
    // one allocation per route name, shared by every row the route expands into
    pub name: Arc<str>,
    pub threshold: f64,
}

// one indexed utterance with the threshold of the first route of its name, so scoring never looks a route up
#[derive(Debug, Clone, PartialEq)]
pub struct UtteranceRow {
    pub route: Arc<str>,
    pub threshold: f64,
    pub text: String,
}

// the immutable route index: rows in the semantic router's sorted utterance order, groups in config order
#[derive(Debug, Clone, PartialEq)]
pub struct RouteIndex {
    pub groups: Vec<RouteGroup>,
    pub rows: Vec<UtteranceRow>,
}

#[derive(Debug)]
struct IndexedRoute {
    name: String,
    utterances: Vec<String>,
    threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    // the encoder keys are accepted as a whole route file's contents, though the inline source never builds an encoder
    #[serde(default, rename = "encoder_type")]
    _encoder_type: Option<IgnoredAny>,
    #[serde(default, rename = "encoder_name")]
    _encoder_name: Option<IgnoredAny>,
    #[serde(default)]
    routes: Vec<RawRoute>,
}

// function_schemas, llm and metadata belong to a dynamic route, which this index never builds, so they are read and dropped
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoute {
    name: String,
    #[serde(default, rename = "description")]
    _description: Option<String>,
    #[serde(default)]
    utterances: Vec<String>,
    #[serde(default)]
    score_threshold: Option<f64>,
    #[serde(default, rename = "function_schemas")]
    _function_schemas: Option<IgnoredAny>,
    #[serde(default, rename = "llm")]
    _llm: Option<IgnoredAny>,
    #[serde(default, rename = "metadata")]
    _metadata: Option<IgnoredAny>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(rename = "encoder_type")]
    _encoder_type: String,
    #[serde(rename = "encoder_name")]
    _encoder_name: String,
    routes: Vec<RawFileRoute>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFileRoute {
    name: String,
    #[serde(default, rename = "description")]
    _description: Option<String>,
    utterances: Vec<String>,
    #[serde(default)]
    score_threshold: Option<f64>,
    #[serde(default, rename = "function_schemas")]
    _function_schemas: Option<IgnoredAny>,
    #[serde(default, rename = "llm")]
    _llm: Option<IgnoredAny>,
    #[serde(default, rename = "metadata")]
    _metadata: Option<IgnoredAny>,
}

fn assemble(routes: Vec<IndexedRoute>) -> Result<RouteIndex, RouteIndexError> {
    let mut groups: Vec<RouteGroup> = Vec::new();
    let mut rows: Vec<UtteranceRow> = Vec::new();
    for route in routes {
        if let Some(threshold) = route.threshold
            && !threshold.is_finite()
        {
            return Err(RouteIndexError::NonFiniteThreshold { name: route.name });
        }
        // a repeated route name keeps the first route's threshold, as the router's first-name lookup does
        let first = groups
            .iter()
            .find(|group| group.name.as_ref() == route.name.as_str())
            .map(|group| (Arc::clone(&group.name), group.threshold));
        let (name, threshold) = if let Some(found) = first {
            found
        } else {
            let threshold = route.threshold.unwrap_or(DEFAULT_SCORE_THRESHOLD);
            let name: Arc<str> = Arc::from(route.name.as_str());
            groups.push(RouteGroup {
                name: Arc::clone(&name),
                threshold,
            });
            (name, threshold)
        };
        // one row per distinct text, so a repeated utterance never turns into a second row or a second name clone
        let mut utterances = route.utterances;
        utterances.sort_unstable();
        utterances.dedup();
        for text in utterances {
            rows.push(UtteranceRow {
                route: Arc::clone(&name),
                threshold,
                text,
            });
        }
    }
    // the sync diff orders utterances by Utterance.to_str(include_metadata=True), not by their two fields
    rows.sort_by(compare_sync_keys);
    rows.dedup();
    Ok(RouteIndex { groups, rows })
}

// the composed sync string compared byte by byte, so a long route name is never copied per row or walked per comparison
fn compare_sync_keys(left: &UtteranceRow, right: &UtteranceRow) -> Ordering {
    // one allocation per route name, so two rows of one route share it and their separator and tail decide alone
    if Arc::ptr_eq(&left.route, &right.route) {
        let left_tail = left.text.bytes().chain(*b" | None | {}");
        let right_tail = right.text.bytes().chain(*b" | None | {}");
        return left_tail.cmp(right_tail);
    }
    let left_key = left
        .route
        .bytes()
        .chain(*b": ")
        .chain(left.text.bytes())
        .chain(*b" | None | {}");
    let right_key = right
        .route
        .bytes()
        .chain(*b": ")
        .chain(right.text.bytes())
        .chain(*b" | None | {}");
    left_key.cmp(right_key)
}

pub fn parse_inline(config: &str) -> Result<RouteIndex, RouteIndexError> {
    let bytes = config.as_bytes();
    if bytes.len() > ROUTE_INDEX_MAX_BYTES {
        return Err(RouteIndexError::TooLarge {
            limit: ROUTE_INDEX_MAX_BYTES,
        });
    }
    let raw: RawConfig = serde_json::from_slice(bytes).map_err(RouteIndexError::Json)?;
    let mut routes = Vec::with_capacity(raw.routes.len());
    for route in raw.routes {
        routes.push(IndexedRoute {
            name: route.name,
            utterances: route.utterances,
            threshold: route.score_threshold,
        });
    }
    assemble(routes)
}

// one read past the bound proves TooLarge without allocating a longer input
fn read_bounded(path: &Path) -> Result<Vec<u8>, RouteIndexError> {
    let file = File::open(path).map_err(|source| RouteIndexError::Read {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|source| RouteIndexError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > ROUTE_INDEX_MAX_BYTES {
        return Err(RouteIndexError::TooLarge {
            limit: ROUTE_INDEX_MAX_BYTES,
        });
    }
    Ok(bytes)
}

pub fn load_file(path: &Path) -> Result<RouteIndex, RouteIndexError> {
    let yaml = match path.extension().and_then(OsStr::to_str) {
        Some("json") => false,
        Some("yaml" | "yml") => true,
        _ => {
            return Err(RouteIndexError::UnsupportedExtension {
                path: path.to_owned(),
            });
        }
    };
    let bytes = read_bounded(path)?;
    let raw: RawFile = if yaml {
        serde_saphyr::from_slice(&bytes).map_err(RouteIndexError::Yaml)?
    } else {
        serde_json::from_slice(&bytes).map_err(RouteIndexError::Json)?
    };
    let mut routes = Vec::with_capacity(raw.routes.len());
    for route in raw.routes {
        routes.push(IndexedRoute {
            name: route.name,
            utterances: route.utterances,
            threshold: route.score_threshold,
        });
    }
    assemble(routes)
}
