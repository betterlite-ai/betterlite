use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

// the index is built once and then only read, so the whole route config stays in memory under one byte bound
pub const ROUTE_INDEX_MAX_BYTES: usize = 1_048_576;
// LiteLLMRouterEncoder's score threshold default, used by every route that sets no threshold of its own
pub const DEFAULT_SCORE_THRESHOLD: f64 = 0.3;
// one byte past the bound, so a file at the limit reads whole while a longer one still proves TooLarge
const READ_LIMIT_BYTES: u64 = 1_048_577;

#[derive(Debug, thiserror::Error)]
pub enum RouteIndexError {
    #[error("the route index input is over the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error("the route config at {} could not be read", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
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
    pub name: String,
    pub threshold: f64,
}

// one indexed utterance with the threshold of the first route of its name, so scoring never looks a route up
#[derive(Debug, Clone, PartialEq)]
pub struct UtteranceRow {
    pub route: String,
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
struct RawConfig {
    #[serde(default)]
    routes: Vec<RawRoute>,
}

#[derive(Debug, Deserialize)]
struct RawRoute {
    name: String,
    #[serde(default, rename = "description")]
    _description: Option<String>,
    #[serde(default)]
    utterances: Vec<String>,
    #[serde(default)]
    score_threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    #[serde(rename = "encoder_type")]
    _encoder_type: String,
    #[serde(rename = "encoder_name")]
    _encoder_name: String,
    routes: Vec<RawFileRoute>,
}

#[derive(Debug, Deserialize)]
struct RawFileRoute {
    name: String,
    #[serde(default, rename = "description")]
    _description: Option<String>,
    utterances: Vec<String>,
    #[serde(default)]
    score_threshold: Option<f64>,
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
        let threshold = match groups.iter().find(|group| group.name == route.name) {
            Some(group) => group.threshold,
            None => {
                let threshold = route.threshold.unwrap_or(DEFAULT_SCORE_THRESHOLD);
                groups.push(RouteGroup {
                    name: route.name.clone(),
                    threshold,
                });
                threshold
            }
        };
        for text in route.utterances {
            rows.push(UtteranceRow {
                route: route.name.clone(),
                threshold,
                text,
            });
        }
    }
    // the sync diff orders utterances by Utterance.to_str(include_metadata=True), not by their two fields
    rows.sort_by_key(|row| format!("{}: {} | None | {{}}", row.route, row.text));
    rows.dedup();
    Ok(RouteIndex { groups, rows })
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
    let file = std::fs::File::open(path).map_err(|source| RouteIndexError::Read {
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
