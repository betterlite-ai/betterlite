use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use soft_canonicalize::soft_canonicalize;
use tempfile::TempDir;

pub(super) struct Layout {
    _root: TempDir,
    pub(super) allowed: PathBuf,
    pub(super) outside: PathBuf,
    pub(super) environment: BTreeMap<String, String>,
}

pub(super) fn environment_allowing(directories: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(
        "LITELLM_OIDC_ALLOWED_CREDENTIAL_DIRS".to_owned(),
        directories.to_owned(),
    )])
}

pub(super) fn layout() -> Layout {
    let root = TempDir::new().unwrap();
    let base = soft_canonicalize(root.path()).unwrap();
    let allowed = base.join("allowed");
    let outside = base.join("outside");
    fs::create_dir_all(allowed.join("nested/deeper")).unwrap();
    fs::create_dir_all(outside.join("sub")).unwrap();
    fs::write(allowed.join("token"), "inside").unwrap();
    fs::write(allowed.join("nested/deeper/token"), "inside").unwrap();
    fs::write(outside.join("secret"), "outside").unwrap();
    let environment = environment_allowing(allowed.to_str().unwrap());
    Layout {
        _root: root,
        allowed,
        outside,
        environment,
    }
}
