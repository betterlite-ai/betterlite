use betterlite_openai::files::records::{FileKind, FileObject, FileStatus, FilesPurpose};
use percent_encoding::percent_decode_str;

use super::{S3SigningError, UPLOAD_CONTENT_LENGTH_PARAM, UPLOAD_URL_PARAM, param_text};
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::request_policy::{Params, RequestValue};

// urllib.parse.uses_params: only these schemes split ;params off the last path segment
const PARAM_SCHEMES: [&str; 16] = [
    "", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu",
    "sip", "sips", "mms", "sftp", "tel",
];

// re.match(r"https://s3\.([^.]+)\.amazonaws\.com/([^/]+)/(.+)")
fn path_style(url: &str) -> Option<(&str, &str, &str)> {
    let (region, after_region) = url.strip_prefix("https://s3.")?.split_once('.')?;
    let (bucket, key) = after_region
        .strip_prefix("amazonaws.com/")?
        .split_once('/')?;
    Some((region, bucket, key))
}

// re.match(r"https://([^.]+)\.s3\.([^.]+)\.amazonaws\.com/(.+)"), so a dotted bucket never matches
fn virtual_host(url: &str) -> Option<(&str, &str, &str)> {
    let (bucket, after_bucket) = url.strip_prefix("https://")?.split_once('.')?;
    let (region, after_region) = after_bucket.strip_prefix("s3.")?.split_once('.')?;
    Some((region, bucket, after_region.strip_prefix("amazonaws.com/")?))
}

// urllib.parse.urlparse(url).path, without the query, fragment and ;params it splits off
fn parsed_path(url: &str) -> Option<String> {
    let cleaned: String = url
        .trim_start_matches(|character: char| character <= ' ')
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let (scheme, after_scheme) = match cleaned.split_once(':') {
        Some((scheme, rest))
            if scheme.starts_with(|character: char| character.is_ascii_alphabetic())
                && scheme.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
                }) =>
        {
            (scheme.to_ascii_lowercase(), rest)
        }
        Some(_) | None => (String::new(), cleaned.as_str()),
    };
    let path = match after_scheme.strip_prefix("//") {
        Some(rest) => {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let (netloc, path) = rest.split_at_checked(end)?;
            if netloc.contains('[') != netloc.contains(']') {
                return None;
            }
            path
        }
        None => after_scheme,
    };
    let without_query = path.split(['?', '#']).next()?;
    let last_segment = without_query.rfind('/').unwrap_or_default();
    let params_at = without_query
        .get(last_segment..)?
        .find(';')
        .map(|offset| last_segment.saturating_add(offset))
        .filter(|_| PARAM_SCHEMES.contains(&scheme.as_str()));
    let end = params_at.unwrap_or(without_query.len());
    without_query.get(..end).map(str::to_owned)
}

pub fn convert_https_url_to_s3_uri(url: &str) -> Result<(String, String), S3SigningError> {
    let unparsable = || S3SigningError::UnparsableUrl {
        url: url.to_owned(),
    };
    // every capture of both patterns is non-empty, and their final . stops at a newline
    let matched = path_style(url).or_else(|| virtual_host(url));
    let regex_match = matched.and_then(|(region, bucket, key)| {
        let capture = key.split('\n').next()?;
        (!region.is_empty() && !bucket.is_empty() && !capture.is_empty())
            .then_some((bucket, capture))
    });
    let (bucket, raw_key) = if let Some((bucket, key)) = regex_match {
        (bucket.to_owned(), key.to_owned())
    } else {
        let path = parsed_path(url).ok_or_else(unparsable)?;
        let (bucket, key) = path
            .trim_start_matches('/')
            .split_once('/')
            .ok_or_else(unparsable)?;
        (bucket.to_owned(), key.to_owned())
    };
    let key = percent_decode_str(&raw_key).decode_utf8_lossy();
    let filename = key.rsplit_once('/').map_or(&*key, |(_, last)| last);
    Ok((format!("s3://{bucket}/{key}"), filename.to_owned()))
}

// S3 answers PutObject with an empty body, so the size B648 recorded from the signed request wins
pub fn uploaded_object_size(
    litellm_params: &Params,
    response_headers: &[(String, String)],
) -> Result<i64, S3SigningError> {
    if let Some(RequestValue::Raw(raw)) = litellm_params.get(UPLOAD_CONTENT_LENGTH_PARAM) {
        if let Ok(recorded) = serde_json::from_str::<i64>(raw) {
            return Ok(recorded);
        }
        // a Python bool is an int, so a recorded true or false counts as 1 or 0
        if let Ok(flag) = serde_json::from_str::<bool>(raw) {
            return Ok(i64::from(flag));
        }
    }
    // httpx joins a repeated header's values with ", "
    let length = response_headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    // str.isdigit over Latin-1 header text: ASCII digits plus the superscripts int() then rejects
    let digit_like = |character: char| {
        character.is_ascii_digit() || matches!(character, '\u{b9}' | '\u{b2}' | '\u{b3}')
    };
    if !length.is_empty() && length.chars().all(digit_like) {
        return length.parse().map_err(S3SigningError::ContentLength);
    }
    Ok(0)
}

pub fn transform_create_file_response(
    litellm_params: &Params,
    response_headers: &[(String, String)],
    created_at: i64,
) -> Result<FileObject, S3SigningError> {
    let upload_url = param_text(litellm_params, UPLOAD_URL_PARAM)?.filter(|url| !url.is_empty());
    let (id, filename) = upload_url
        .as_deref()
        .map(convert_https_url_to_s3_uri)
        .transpose()?
        .unwrap_or_default();
    let bytes = uploaded_object_size(litellm_params, response_headers)?;
    Ok(FileObject {
        id,
        bytes,
        created_at,
        filename,
        object: FileKind::File,
        purpose: FilesPurpose::Batch,
        status: Some(FileStatus::Uploaded),
        expires_at: None,
        status_details: None,
    })
}

pub fn get_error_class(
    status_code: u16,
    message: String,
    headers: ErrorHeaders,
) -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(headers),
        response: None,
    })
}
