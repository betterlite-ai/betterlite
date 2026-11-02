use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use soft_canonicalize::soft_canonicalize;

const LOCAL_IMAGE_HEADER_BYTES: u64 = 512;

pub trait AssetLogger {
    fn debug(&self, message: &str);
    fn warning(&self, message: &str);
}

pub struct ResolvedImage {
    pub path: PathBuf,
    pub media_type: &'static str,
}

#[must_use]
pub fn detect_local_image_media_type(header: &[u8]) -> Option<&'static str> {
    if header.get(0..8) == Some(b"\x89PNG\r\n\x1a\n".as_slice()) {
        return Some("image/png");
    }
    if header.get(0..4) == Some(b"GIF8".as_slice()) && header.get(5..6) == Some(b"a".as_slice()) {
        return Some("image/gif");
    }
    if header.get(0..3) == Some(b"\xff\xd8\xff".as_slice()) {
        return Some("image/jpeg");
    }
    if header.get(0..4) == Some(b"RIFF".as_slice()) && header.get(8..12) == Some(b"WEBP".as_slice())
    {
        return Some("image/webp");
    }
    let icon_header = header.get(0..4);
    if icon_header == Some(b"\x00\x00\x01\x00".as_slice())
        || icon_header == Some(b"\x00\x00\x02\x00".as_slice())
    {
        return Some("image/x-icon");
    }
    None
}

pub fn resolve_local_image<L>(
    candidate: &str,
    home_dir: Option<&Path>,
    logger: &L,
) -> Option<ResolvedImage>
where
    L: AssetLogger,
{
    if candidate.is_empty() {
        return None;
    }
    let expanded = expand_home(candidate, home_dir);
    let resolved = soft_canonicalize(&expanded).ok()?;
    if !resolved.is_file() {
        return None;
    }
    let header = match read_header(&resolved) {
        Ok(bytes) => bytes,
        Err(error) => {
            logger.debug(&format!(
                "Could not read local asset {candidate:?}: {error}"
            ));
            return None;
        }
    };
    let Some(media_type) = detect_local_image_media_type(&header) else {
        logger.warning(&format!(
            "Local asset {candidate:?} is not a supported image file; falling back to default."
        ));
        return None;
    };
    Some(ResolvedImage {
        path: resolved,
        media_type,
    })
}

fn expand_home(candidate: &str, home_dir: Option<&Path>) -> PathBuf {
    let Some(home) = home_dir else {
        return PathBuf::from(candidate);
    };
    if candidate == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = candidate.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(candidate)
}

fn read_header(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut buffer = Vec::new();
    file.take(LOCAL_IMAGE_HEADER_BYTES)
        .read_to_end(&mut buffer)?;
    Ok(buffer)
}

mod tests;
