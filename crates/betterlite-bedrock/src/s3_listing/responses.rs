use std::collections::BTreeMap;

use betterlite_context::cloud_storage::split_bucket_name;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_openai::files::records::{
    FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose,
};

use super::decode::{DecodeLimits, S3ListedEntry, decode_s3_listing, parse_created_at, parse_size};
use super::prefix::{
    BATCH_OUTPUT_LISTING_PREFIX, listing_bucket_name, setting_text, walked_listing_purpose,
};
use super::{LIST_FILES_PURPOSE_PARAM, S3ListingError};
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, ErrorReply, ResponseBody};
use crate::file_content::{
    FileContentError, FileContentRequest, RawResponse, transform_file_response,
};
use crate::request_policy::Params;
use crate::s3_access::S3AccessParams;
use crate::s3_access::managed::validate_configured_file_id;

#[derive(Debug, Clone, Copy)]
pub struct S3ResponseRequest<'input> {
    pub method: &'input str,
    pub url: &'input str,
}

#[derive(Clone, Copy)]
pub struct S3RawResponse<'input> {
    pub status: u16,
    pub body: &'input [u8],
    pub headers: &'input [(String, String)],
    pub request: S3ResponseRequest<'input>,
}

#[derive(Clone, Copy)]
pub struct S3ResponseContext<'input> {
    pub litellm_params: &'input Params,
    pub access: &'input S3AccessParams,
    pub environment: &'input BTreeMap<String, String>,
}

// httpx decodes the body by its declared charset; reusing B646's transform keeps that text
fn response_text(raw: &S3RawResponse<'_>) -> Result<String, S3ListingError> {
    let decoded = transform_file_response(RawResponse {
        status_code: 400,
        body: raw.body.to_vec(),
        headers: raw.headers.to_vec(),
        request: FileContentRequest {
            method: raw.request.method.to_owned(),
            url: raw.request.url.to_owned(),
        },
    });
    match decoded {
        // a forced 400 always takes the error branch, whose message is the decoded response text
        Err(FileContentError::Bedrock { error, .. }) => Ok(error.cause.error.message.clone()),
        Err(FileContentError::Header(header)) => Err(S3ListingError::Header(header)),
        Ok(_) => Ok(String::new()),
    }
}

fn list_reply(raw: &S3RawResponse<'_>, message: &str) -> ErrorReply {
    ErrorReply {
        response: ErrorResponse {
            status_code: raw.status,
            request: ErrorRequest {
                method: raw.request.method.to_owned(),
                url: raw.request.url.to_owned(),
            },
        },
        headers: raw.headers.to_vec(),
        body: ResponseBody::Read(message.to_owned()),
    }
}

fn bedrock_error(
    raw: &S3RawResponse<'_>,
    message: String,
    status: u16,
    reply: Option<ErrorReply>,
) -> Result<BedrockError, S3ListingError> {
    BedrockError::try_from(ErrorInput {
        status_code: status,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(raw.headers.to_vec())),
        response: reply,
    })
    .map_err(S3ListingError::Header)
}

fn listing_purpose_matches(file: &FileObject, requested: Option<&str>) -> bool {
    match requested {
        None => true,
        Some("batch") => file.purpose == FilesPurpose::Batch,
        Some("batch_output") => file.purpose == FilesPurpose::BatchOutput,
        Some(_) => false,
    }
}

// authorization failure omits the entry without a diagnostic, as LiteLLM's except ValueError does
fn listed_managed_file(
    entry: &S3ListedEntry,
    bucket_name: &str,
    configured_bucket_name: &str,
    allow_legacy: bool,
) -> Result<Option<FileObject>, S3ListingError> {
    let object_key = entry.key.as_deref().unwrap_or_default();
    if object_key.is_empty() {
        return Ok(None);
    }
    let file_id = format!("s3://{bucket_name}/{object_key}");
    let configured = [configured_bucket_name.to_owned()];
    if validate_configured_file_id(&file_id, &configured, allow_legacy).is_err() {
        return Ok(None);
    }
    let (_, configured_prefix) = split_bucket_name(configured_bucket_name)?;
    let relative_key = if configured_prefix.is_empty() {
        object_key.to_owned()
    } else {
        let skip = configured_prefix.chars().count().saturating_add(1);
        object_key.chars().skip(skip).collect()
    };
    let purpose = if relative_key.starts_with(BATCH_OUTPUT_LISTING_PREFIX) {
        FilesPurpose::BatchOutput
    } else {
        FilesPurpose::Batch
    };
    Ok(Some(FileObject {
        id: file_id,
        bytes: parse_size(entry.size.as_deref())?,
        created_at: parse_created_at(entry.last_modified.as_deref())?,
        filename: object_key
            .rsplit_once('/')
            .map_or_else(|| object_key.to_owned(), |(_head, tail)| tail.to_owned()),
        object: FileKind::File,
        purpose,
        status: Some(FileStatus::Uploaded),
        expires_at: None,
        status_details: None,
    }))
}

pub fn list_files_response(
    raw: &S3RawResponse<'_>,
    context: &S3ResponseContext<'_>,
) -> Result<Vec<FileObject>, S3ListingError> {
    if raw.status >= 400 {
        let message = response_text(raw)?;
        let reply = list_reply(raw, &message);
        let error = bedrock_error(raw, message, raw.status, Some(reply))?;
        return Err(S3ListingError::Bedrock(Box::new(error)));
    }
    let requested = setting_text(context.litellm_params, LIST_FILES_PURPOSE_PARAM);
    let walked = walked_listing_purpose(context.litellm_params);
    let configured_bucket_name =
        listing_bucket_name(context.access, context.environment, walked.as_deref())?;
    let allow_legacy = context.access.trusted.allows_legacy();
    let listing = decode_s3_listing(raw.body, &DecodeLimits::DEFAULT)?;
    let bucket_name = match listing.name {
        Some(name) => name,
        None => split_bucket_name(&configured_bucket_name)?.0,
    };
    let mut files = Vec::new();
    for entry in &listing.contents {
        let Some(file) =
            listed_managed_file(entry, &bucket_name, &configured_bucket_name, allow_legacy)?
        else {
            continue;
        };
        if listing_purpose_matches(&file, requested.as_deref()) {
            files.push(file);
        }
    }
    Ok(files)
}

pub fn delete_file_response(
    raw: &S3RawResponse<'_>,
    additional_args: Option<&Params>,
) -> Result<FileDeleted, S3ListingError> {
    if raw.status != 204 {
        let decoded = response_text(raw)?;
        let message = if decoded.is_empty() {
            format!("S3 file deletion returned HTTP {}", raw.status)
        } else {
            decoded
        };
        let status = if raw.status >= 400 { raw.status } else { 502 };
        let error = bedrock_error(raw, message, status, None)?;
        return Err(S3ListingError::Bedrock(Box::new(error)));
    }
    let requested_id = additional_args.and_then(|args| setting_text(args, "file_id"));
    let Some(file_id) = requested_id.filter(|value| !value.is_empty()) else {
        return Err(S3ListingError::DeleteContext);
    };
    Ok(FileDeleted {
        id: file_id,
        object: FileKind::File,
        deleted: true,
    })
}
