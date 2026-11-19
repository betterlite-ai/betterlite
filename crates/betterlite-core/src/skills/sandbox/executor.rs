use std::time::Duration;

use betterlite_pyhost::owned_temp::{OwnedTempDir, TempError};
use thiserror::Error;
use tokio::time::timeout;

use crate::api::skills::hook::execution::SkillFiles;

use super::collector::{GeneratedFile, collect_generated_files};
use super::programs::{
    REQUIREMENTS_FILE, SANDBOX_ROOT, SESSION_LANG, SKILL_REQUIREMENTS_FILE, pip_install_code,
    wrapped_code,
};
use super::session::{RunResult, SandboxSession, SessionError, SessionOpener, SessionOptions};
use super::staging::{StageError, stage_file};

const SDK_MISSING: &str = "llm-sandbox not installed. Install `llm-sandbox`.";

pub struct SandboxRequest<'a> {
    pub code: &'a str,
    pub skill_files: &'a SkillFiles,
    pub requirements: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxOutcome {
    pub success: bool,
    pub output: String,
    pub error: String,
    pub files: Vec<GeneratedFile>,
}

pub struct SandboxExecutor<S> {
    pub opener: S,
    pub image: Option<String>,
    pub timeout: Duration,
}

#[derive(Debug, Error)]
enum DriveError {
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Stage(#[from] StageError),
    #[error(transparent)]
    Temp(#[from] TempError),
}

impl<S: SessionOpener> SandboxExecutor<S> {
    pub async fn execute(&self, request: SandboxRequest<'_>) -> SandboxOutcome {
        match timeout(self.timeout, self.run(request)).await {
            Ok(outcome) => outcome,
            Err(_elapsed) => SandboxOutcome {
                success: false,
                output: String::new(),
                error: format!(
                    "the sandbox execution did not finish within {}s",
                    self.timeout.as_secs()
                ),
                files: Vec::new(),
            },
        }
    }

    async fn run(&self, request: SandboxRequest<'_>) -> SandboxOutcome {
        let options = SessionOptions {
            lang: SESSION_LANG,
            verbose: false,
            image: self.image.clone().filter(|image| !image.is_empty()),
        };
        let session = match self.opener.open(&options).await {
            Ok(session) => session,
            Err(SessionError::SdkMissing) => {
                return SandboxOutcome {
                    success: false,
                    output: String::new(),
                    error: SDK_MISSING.to_owned(),
                    files: Vec::new(),
                };
            }
            Err(error) => {
                return SandboxOutcome {
                    success: false,
                    output: String::new(),
                    error: error.to_string(),
                    files: Vec::new(),
                };
            }
        };
        let outcome = match drive(session.as_ref(), request).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::error!("SkillsSandboxExecutor: Execution failed: {error}");
                SandboxOutcome {
                    success: false,
                    output: String::new(),
                    error: error.to_string(),
                    files: Vec::new(),
                }
            }
        };
        // the source returns inside `with`, so a failing exit replaces the result it had built
        match session.close().await {
            Ok(()) => outcome,
            Err(error) => {
                tracing::error!("SkillsSandboxExecutor: Execution failed: {error}");
                SandboxOutcome {
                    success: false,
                    output: String::new(),
                    error: error.to_string(),
                    files: Vec::new(),
                }
            }
        }
    }
}

async fn drive(
    session: &dyn SandboxSession,
    request: SandboxRequest<'_>,
) -> Result<SandboxOutcome, DriveError> {
    let staging = OwnedTempDir::new()?;
    for (path, content) in request.skill_files {
        let Some(local) = stage_file(staging.path(), path, content).await? else {
            tracing::warn!("SkillsSandboxExecutor: Skipping file with invalid path: {path}");
            continue;
        };
        session
            .copy_to_runtime(&local, &format!("{SANDBOX_ROOT}/{path}"))
            .await?;
    }
    if let Some(result) = install_requirements(session, &request).await? {
        if result.exit_code != 0 {
            return Ok(SandboxOutcome {
                success: false,
                output: result.stdout.unwrap_or_default(),
                error: result.stderr.unwrap_or_default(),
                files: Vec::new(),
            });
        }
    }
    let result = session.run(&wrapped_code(request.code)).await?;
    let downloads = OwnedTempDir::new()?;
    let files = collect_generated_files(session, request.skill_files, downloads.path()).await;
    Ok(SandboxOutcome {
        success: result.exit_code == 0,
        output: result.stdout.unwrap_or_default(),
        error: result.stderr.unwrap_or_default(),
        files,
    })
}

async fn install_requirements(
    session: &dyn SandboxSession,
    request: &SandboxRequest<'_>,
) -> Result<Option<RunResult>, DriveError> {
    let filename = match request.requirements {
        Some(text) if !text.is_empty() => {
            let folder = OwnedTempDir::new()?;
            let local = folder.path().join(SKILL_REQUIREMENTS_FILE);
            tokio::fs::write(&local, text)
                .await
                .map_err(|source| StageError::Write {
                    path: local.clone(),
                    source,
                })?;
            session
                .copy_to_runtime(&local, &format!("{SANDBOX_ROOT}/{REQUIREMENTS_FILE}"))
                .await?;
            REQUIREMENTS_FILE
        }
        _ if request.skill_files.contains_key(SKILL_REQUIREMENTS_FILE) => SKILL_REQUIREMENTS_FILE,
        _ => return Ok(None),
    };
    let result = session.run(&pip_install_code(filename)).await?;
    Ok(Some(result))
}
