use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::MAIN_SEPARATOR;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::errors::HarnessError;
use crate::payloads::read_skill_files;
use crate::sandbox::{Sandbox, SandboxCommand};
use crate::transformation::HarnessSessionSetup;

// links <private_dir>/<link> to a betterlite-owned cache dir so a later session can resume
const PERSIST_DIR_SCRIPT: &str = "d=\"${HOME:-/tmp}/.cache/litellm-harness/$2\"; mkdir -p \"$d\" && mkdir -p \"$(dirname \"$1\")\" && ln -sfn \"$d\" \"$1\"";
const PERSIST_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const PERSIST_OUTPUT_LIMIT: usize = 16 * 1024;

#[must_use]
pub(super) fn sandbox_path(private_dir: &str, path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{private_dir}/{path}")
    }
}

pub(super) async fn persist_dir<S>(
    sandbox: &S,
    link_path: &str,
    cache_subpath: &str,
) -> Result<(), HarnessError>
where
    S: Sandbox,
{
    let command = SandboxCommand {
        argv: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            PERSIST_DIR_SCRIPT.to_owned(),
            "sh".to_owned(),
            link_path.to_owned(),
            cache_subpath.to_owned(),
        ],
        env: BTreeMap::new(),
        cwd: None,
    };
    let result = sandbox
        .run(&command, PERSIST_TIMEOUT, PERSIST_OUTPUT_LIMIT)
        .await?;
    if result.exit_code != 0_i32 {
        tracing::debug!(
            cache_subpath,
            stderr = %result.stderr,
            "harness: could not persist cache dir, resume across sessions disabled",
        );
    }
    Ok(())
}

async fn load_skill_files(skill: &str) -> Result<(String, Vec<(String, Vec<u8>)>), HarnessError> {
    let resolved = fs::canonicalize(skill).map_err(|source| HarnessError::Sandbox { source })?;
    let name = resolved
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| HarnessError::Sandbox {
            source: io::Error::new(io::ErrorKind::InvalidData, "skill path is not valid UTF-8"),
        })?;
    let mut walks = JoinSet::new();
    walks.spawn_blocking(move || read_skill_files(&resolved));
    let files = match walks.join_next().await {
        Some(Ok(files)) => files.map_err(|source| HarnessError::Sandbox {
            source: io::Error::other(source),
        })?,
        Some(Err(source)) => {
            return Err(HarnessError::Sandbox {
                source: io::Error::other(source),
            });
        }
        None => Vec::new(),
    };
    Ok((name, files))
}

pub(super) async fn copy_skills<S>(
    sandbox: &S,
    skills: &[String],
    skills_root: &str,
) -> Result<(), HarnessError>
where
    S: Sandbox,
{
    for skill in skills {
        let (name, files) = load_skill_files(skill).await?;
        for (relative, data) in files {
            let normalized = relative.replace(MAIN_SEPARATOR, "/");
            sandbox
                .write(&format!("{skills_root}/{name}/{normalized}"), &data)
                .await?;
        }
    }
    Ok(())
}

// writes a session's persisted-dir links, files, and skills in the order `start()` needs them
pub(super) async fn apply_session_setup<S>(
    sandbox: &S,
    private_dir: &str,
    setup: &HarnessSessionSetup,
    skills: &[String],
) -> Result<(), HarnessError>
where
    S: Sandbox,
{
    for persisted in &setup.persisted_dirs {
        persist_dir(
            sandbox,
            &sandbox_path(private_dir, &persisted.private_path),
            &persisted.cache_path,
        )
        .await?;
    }
    for (path, data) in &setup.files {
        sandbox
            .write(&sandbox_path(private_dir, path), data)
            .await?;
    }
    if let Some(skills_dir) = setup.skills_dir.as_deref().filter(|_| !skills.is_empty()) {
        copy_skills(sandbox, skills, &sandbox_path(private_dir, skills_dir)).await?;
    }
    Ok(())
}
