use serde::{Deserialize, Serialize};

use super::bullets::count_release_bullets;

pub const FIXED_RELEASE_URL: &str =
    "https://api.github.com/repos/betterlite-ai/betterlite/releases/latest";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct LatestReleaseInfo {
    pub version: String,
    pub new_features: u64,
    pub bug_fixes: u64,
    pub other_updates: u64,
    pub release_url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct LatestReleaseUnavailable {
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum LatestRelease {
    Available(Box<LatestReleaseInfo>),
    Unavailable(Box<LatestReleaseUnavailable>),
}

#[must_use]
pub fn decode_latest_release(status: u16, body: &[u8]) -> LatestRelease {
    if status != 200 {
        return LatestRelease::Unavailable(Box::new(LatestReleaseUnavailable {
            reason: format!("GitHub responded with status {status}"),
        }));
    }
    let release: GitHubRelease = match serde_json::from_slice(body) {
        Ok(release) => release,
        Err(error) => {
            return LatestRelease::Unavailable(Box::new(LatestReleaseUnavailable {
                reason: format!("GitHub release payload was not the expected shape: {error}"),
            }));
        }
    };
    let counts = count_release_bullets(&release.body);
    LatestRelease::Available(Box::new(LatestReleaseInfo {
        version: release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name)
            .to_owned(),
        new_features: counts.new_features,
        bug_fixes: counts.bug_fixes,
        other_updates: counts.other_updates,
        release_url: release.html_url,
    }))
}

// The fuzzer packs an arbitrary response status into the first byte.
#[must_use]
pub fn decode_fuzzed_latest_release(bytes: &[u8]) -> LatestRelease {
    let (status, body) = bytes
        .split_first()
        .map_or((200_u16, bytes), |(status, body)| {
            (u16::from(*status), body)
        });
    decode_latest_release(status, body)
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    body: String,
}
