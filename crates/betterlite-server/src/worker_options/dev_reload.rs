use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use betterlite_config::sections::worker_options_reload::ReloadSettings;

const ENV_OVERRIDE_WARNING: &str = "LiteLLM --reload: worker processes re-read .env with override, so .env values win over shell-exported environment variables. Unset a key in .env to let a shell-exported value take precedence.";
const PYTHON_GLOB: &str = "*.py";
const ENV_FILE: &str = ".env";

#[derive(Debug, thiserror::Error)]
pub enum ReloadError {
    #[error("cannot resolve {path}: {source}")]
    Resolve {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the config file name {path} is not valid UTF-8")]
    FileName { path: PathBuf },
}

fn absolute(cwd: &Path, path: &Path) -> Result<PathBuf, ReloadError> {
    // join keeps an absolute path as it is, the way os.path.abspath does, so a relative one lands under the cwd
    let candidate = cwd.join(path);
    soft_canonicalize::soft_canonicalize(&candidate).map_err(|source| ReloadError::Resolve {
        path: candidate,
        source,
    })
}

pub fn resolve_reload_options(
    cwd: &Path,
    config: Option<&Path>,
) -> Result<ReloadSettings, ReloadError> {
    let watch_root = absolute(cwd, cwd)?;
    let mut reload_dirs = vec![watch_root.clone()];
    let mut reload_includes = vec![PYTHON_GLOB.to_owned(), ENV_FILE.to_owned()];
    if let Some(path) = config {
        // abspath joins the resolved current directory and keeps a symlink in the config path as it is
        let config_abs = watch_root.join(path);
        if let Some(parent) = config_abs.parent()
            && parent != watch_root.as_path()
        {
            reload_dirs.push(parent.to_path_buf());
        }
        let file_name = config_abs
            .file_name()
            .and_then(OsStr::to_str)
            .map(str::to_owned);
        let Some(name) = file_name else {
            return Err(ReloadError::FileName { path: config_abs });
        };
        reload_includes.push(name);
    }
    Ok(ReloadSettings {
        reload: true,
        reload_dirs,
        reload_includes,
        extra_paths: BTreeSet::new(),
        env_hot_reload: false,
    })
}

pub fn patch_reload_extra_paths(
    settings: &mut ReloadSettings,
    cwd: &Path,
    extra: &[Option<&Path>],
) -> Result<bool, ReloadError> {
    let mut patched = false;
    for path in extra.iter().flatten() {
        settings.extra_paths.insert(absolute(cwd, path)?);
        patched = true;
    }
    Ok(patched)
}

pub fn configure_dev_reload(
    settings: &mut ReloadSettings,
    cwd: &Path,
    config: Option<&Path>,
    warn: &mut dyn FnMut(&str),
) -> Result<(), ReloadError> {
    let options = resolve_reload_options(cwd, config)?;
    let env_file = cwd.join(ENV_FILE);
    settings.reload = options.reload;
    settings.reload_dirs = options.reload_dirs;
    settings.reload_includes = options.reload_includes;
    settings.env_hot_reload = true;
    patch_reload_extra_paths(settings, cwd, &[config, Some(env_file.as_path())])?;
    warn(ENV_OVERRIDE_WARNING);
    Ok(())
}
