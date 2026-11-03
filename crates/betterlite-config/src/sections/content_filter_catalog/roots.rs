use std::path::{Path, PathBuf};
use std::{env, io};

#[derive(Debug, thiserror::Error)]
pub enum ContentFilterRootsError {
    #[error("failed to locate current executable: {source}")]
    CurrentExecutable {
        #[source]
        source: io::Error,
    },
    #[error("content-filter assets are not installed relative to executable '{executable}'")]
    AssetsNotInstalled { executable: PathBuf },
}

pub const DATA_DIR: &str = "assets/content_filter";
pub const CATEGORIES_DIR: &str = "assets/content_filter/categories";
pub const POLICY_TEMPLATES_DIR: &str = "assets/content_filter/policy_templates";
pub const LEGACY_DATA_DIR: &str = "assets/content_filter_legacy";

const SHARE_DATA_DIR: &str = "../share/betterlite/content_filter";
const SHARE_LEGACY_DATA_DIR: &str = "../share/betterlite/content_filter_legacy";

pub fn data_roots() -> Result<[PathBuf; 2], ContentFilterRootsError> {
    let executable = env::current_exe()
        .map_err(|source| ContentFilterRootsError::CurrentExecutable { source })?;
    data_roots_from_executable(&executable)
}

fn data_roots_from_executable(executable: &Path) -> Result<[PathBuf; 2], ContentFilterRootsError> {
    let executable_dir =
        executable
            .parent()
            .ok_or_else(|| ContentFilterRootsError::AssetsNotInstalled {
                executable: executable.to_path_buf(),
            })?;
    let layouts = [
        (
            executable_dir.join(DATA_DIR),
            executable_dir.join(LEGACY_DATA_DIR),
        ),
        (
            executable_dir.join(SHARE_DATA_DIR),
            executable_dir.join(SHARE_LEGACY_DATA_DIR),
        ),
    ];
    for (bundled, legacy) in layouts {
        if bundled.is_dir() {
            return Ok([bundled, legacy]);
        }
    }
    for ancestor in executable_dir.ancestors() {
        let package_dir = ancestor.join("crates/betterlite-config");
        let bundled = package_dir.join(DATA_DIR);
        if bundled.is_dir() {
            return Ok([bundled, package_dir.join(LEGACY_DATA_DIR)]);
        }
    }
    Err(ContentFilterRootsError::AssetsNotInstalled {
        executable: executable.to_path_buf(),
    })
}

mod tests;
