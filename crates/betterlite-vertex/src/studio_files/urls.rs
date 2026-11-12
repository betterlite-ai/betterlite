use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::url_path::encode_segment;
use betterlite_openai::files::records::{FileDeleted, FileKind};

use super::error::StudioFilesError;
use super::{RequestParams, resolve_base, resolve_key};
use crate::studio_identity::API_VERSION;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTarget {
    pub url: String,
    pub query: Vec<(String, String)>,
}

// A file ID may be a full URL, but only its path names the file: the host always comes from the base.
pub fn normalize_file_id(file_id: &str) -> Result<String, StudioFilesError> {
    let candidate = if file_id.starts_with("http://") || file_id.starts_with("https://") {
        url_path(file_id)
    } else {
        file_id
    };
    let trimmed = candidate.trim_matches('/');
    let body = trimmed.strip_prefix("files/").unwrap_or(trimmed);
    let encoded = encode_segment(body, "file_id")?;
    Ok(format!("files/{encoded}"))
}

// urllib.parse.urlparse's path stops at the query, fragment or the last segment's ';params'.
fn url_path(url: &str) -> &str {
    let Some((_scheme, rest)) = url.split_once("://") else {
        return "";
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return "";
    };
    if authority.contains(['?', '#']) {
        return "";
    }
    let Some(end) = path.find(['?', '#']) else {
        return without_params(path);
    };
    let trimmed = path.get(..end).unwrap_or(path);
    without_params(trimmed)
}

fn without_params(path: &str) -> &str {
    let segment_start = path.rfind('/').map_or(0, |slash| slash.saturating_add(1));
    let Some((_head, last)) = path.split_at_checked(segment_start) else {
        return path;
    };
    let Some((segment, _params)) = last.split_once(';') else {
        return path;
    };
    let keep = segment_start.saturating_add(segment.len());
    path.get(..keep).unwrap_or(path)
}

// Only retrieve trims the resolved base; delete keeps it as the base helper returns it.
pub fn target(
    file_id: &str,
    params: &RequestParams,
    settings: &Settings,
    trim_base: bool,
) -> Result<FileTarget, StudioFilesError> {
    require_key(params, settings)?;
    let resolved = resolve_base(params.api_base.as_deref(), settings);
    let base = if trim_base {
        resolved.trim_end_matches('/')
    } else {
        resolved.as_str()
    };
    let normalized = normalize_file_id(file_id)?;
    Ok(FileTarget {
        url: format!("{base}/{API_VERSION}/{normalized}"),
        query: Vec::new(),
    })
}

pub fn decode_delete_response(
    status: u16,
    url: Option<&str>,
    body: &[u8],
) -> Result<FileDeleted, StudioFilesError> {
    if status != 200 {
        let text = str::from_utf8(body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", body.len()),
            str::to_owned,
        );
        return Err(StudioFilesError::DeleteFailed { status, body: text });
    }
    Ok(FileDeleted {
        id: deleted_id(url),
        object: FileKind::File,
        deleted: true,
    })
}

// LiteLLM reads the last /files/ segment of the request URL, before the query, and prefixes one.
fn deleted_id(url: Option<&str>) -> String {
    let Some((_before, after)) = url.and_then(|value| value.rsplit_once("/files/")) else {
        return "deleted".to_owned();
    };
    let id = after.split_once('?').map_or(after, |(id, _query)| id);
    if id.starts_with("files/") {
        id.to_owned()
    } else {
        format!("files/{id}")
    }
}

fn require_key(params: &RequestParams, settings: &Settings) -> Result<(), StudioFilesError> {
    let candidates = [params.api_key.as_ref()];
    if resolve_key(&candidates, settings).is_none() {
        return Err(StudioFilesError::MissingApiKey);
    }
    Ok(())
}
