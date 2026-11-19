use std::io;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::api::skills::hook::execution::SkillFiles;

use super::decoder::{
    ListingRefusal, MAX_LISTING_BYTES, MAX_LISTING_ENTRIES, decode_generated_paths,
};
use super::programs::{LISTING_CODE, SANDBOX_PREFIX, mime_type};
use super::session::SandboxSession;

pub const MAX_COLLECTED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub name: String,
    pub path: String,
    pub content_base64: String,
    pub mime_type: String,
}

pub async fn collect_generated_files(
    session: &dyn SandboxSession,
    original: &SkillFiles,
    scratch: &Path,
) -> Vec<GeneratedFile> {
    let listing = match session.run(LISTING_CODE).await {
        Ok(listing) => listing,
        Err(error) => {
            tracing::warn!("SkillsSandboxExecutor: Error collecting generated files: {error}");
            return Vec::new();
        }
    };
    let Some(stdout) = listing.stdout else {
        return Vec::new();
    };
    if listing.exit_code != 0 || stdout.is_empty() {
        return Vec::new();
    }
    let decoded = decode_generated_paths(&stdout);
    if let Some(refusal) = decoded.refusal {
        warn_refusal(refusal);
    }
    copy_generated_files(session, original, scratch, &decoded.paths).await
}

fn warn_refusal(refusal: ListingRefusal) {
    match refusal {
        ListingRefusal::TooLarge => tracing::warn!(
            "SkillsSandboxExecutor: The generated file listing is past {MAX_LISTING_BYTES} bytes, so nothing is collected"
        ),
        ListingRefusal::Malformed => tracing::warn!(
            "SkillsSandboxExecutor: The generated file listing is not a JSON list of paths, so nothing is collected"
        ),
        ListingRefusal::TooManyEntries => tracing::warn!(
            "SkillsSandboxExecutor: The generated file listing is past {MAX_LISTING_ENTRIES} entries, so only the first ones are collected"
        ),
    }
}

async fn copy_generated_files(
    session: &dyn SandboxSession,
    original: &SkillFiles,
    scratch: &Path,
    paths: &[String],
) -> Vec<GeneratedFile> {
    let mut files = Vec::new();
    let mut collected: u64 = 0;
    for (index, path) in paths.iter().enumerate() {
        let Some(relative) = generated_relative(path, original) else {
            continue;
        };
        let destination = scratch.join(format!("collected-{index}"));
        if let Err(error) = session.copy_from_runtime(path, &destination).await {
            tracing::warn!("SkillsSandboxExecutor: Error copying file {path}: {error}");
            continue;
        }
        match content_of(&destination, collected).await {
            Ok(Some(content)) => {
                collected =
                    collected.saturating_add(u64::try_from(content.len()).unwrap_or(u64::MAX));
                files.push(GeneratedFile {
                    name: path.rsplit('/').next().unwrap_or_default().to_owned(),
                    path: relative,
                    content_base64: STANDARD.encode(&content),
                    mime_type: mime_type(path).to_owned(),
                });
            }
            Ok(None) => break,
            Err(error) => {
                tracing::warn!("SkillsSandboxExecutor: Error copying file {path}: {error}");
            }
        }
    }
    files
}

fn generated_relative(path: &str, original: &SkillFiles) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let relative = path.replace(SANDBOX_PREFIX, "");
    if original.contains_key(&relative) {
        return None;
    }
    Some(relative)
}

// None ends the walk so the collected prefix stays under the byte cap
pub(super) async fn content_of(path: &Path, collected: u64) -> Result<Option<Vec<u8>>, io::Error> {
    let metadata = tokio::fs::metadata(path).await?;
    if collected.saturating_add(metadata.len()) > MAX_COLLECTED_BYTES {
        tracing::warn!(
            "SkillsSandboxExecutor: The collected files are past {MAX_COLLECTED_BYTES} bytes, so the rest is dropped"
        );
        return Ok(None);
    }
    tokio::fs::read(path).await.map(Some)
}
