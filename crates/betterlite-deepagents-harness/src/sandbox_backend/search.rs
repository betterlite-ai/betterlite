use betterlite_harness::sandbox::{CompletedRun, Sandbox};
use regex::Regex;

use crate::sandbox_backend::errors::BackendError;
use crate::sandbox_backend::include_glob::IncludeGlob;
use crate::sandbox_backend::protocol::{
    FileInfo, GlobResult, GrepMatch, GrepResult, Listing, Reply,
};
use crate::sandbox_backend::{
    EXIT_XARGS_BATCH, FS_TIMEOUT_SECONDS, GREP_LINE, GREP_SCRIPT, SKIP_DIRS, SandboxBackend,
};

struct GrepRun {
    root: String,
    include: Option<IncludeGlob>,
    line: Regex,
    done: CompletedRun,
}

pub(crate) async fn glob<S>(backend: &SandboxBackend<S>, pattern: &str, path: &str) -> GlobResult
where
    S: Sandbox,
{
    let (include, root, done) = match glob_run(backend, pattern, path).await {
        Ok(found) => found,
        Err(error) => return Reply::Error(error.to_string()),
    };
    if done.exit_code != 0_i32 && done.stdout.is_empty() {
        return Reply::Done(Listing {
            matches: Vec::new(),
            truncated: false,
        });
    }
    let mut files: Vec<FileInfo> = done
        .stdout
        .lines()
        .filter(|real| include.matches(&root, real))
        .map(|real| FileInfo {
            path: backend.to_virtual(real),
            is_dir: false,
        })
        .collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Reply::Done(Listing {
        matches: files,
        truncated: done.exit_code != 0_i32,
    })
}

pub(crate) async fn grep<S>(
    backend: &SandboxBackend<S>,
    pattern: &str,
    path: &str,
    glob: Option<&str>,
    max_count: Option<usize>,
) -> GrepResult
where
    S: Sandbox,
{
    let shown = if path.is_empty() { "/" } else { path };
    let run = match grep_run(backend, pattern, path, glob).await {
        Ok(run) => run,
        Err(error) => return Reply::Error(format!("Path '{shown}': {error}")),
    };
    if let Some(reason) = grep_failure(&run.done) {
        return Reply::Error(format!("Path '{shown}': {reason}"));
    }
    let mut found: Vec<GrepMatch> = run
        .done
        .stdout
        .lines()
        .filter_map(|line| run.line.captures(line))
        .filter_map(|parts| {
            let line = parts.get(2)?.as_str().parse().ok()?;
            Some((parts.get(1)?.as_str(), line, parts.get(3)?.as_str()))
        })
        .filter(|(real, _, _)| {
            run.include
                .as_ref()
                .is_none_or(|include| include.matches(&run.root, real))
        })
        .map(|(real, line, text)| GrepMatch {
            path: backend.to_virtual(real),
            line,
            text: text.to_owned(),
        })
        .collect();
    let truncated = max_count.is_some_and(|max| found.len() > max);
    found.truncate(max_count.unwrap_or(found.len()));
    Reply::Done(Listing {
        matches: found,
        truncated,
    })
}

async fn glob_run<S>(
    backend: &SandboxBackend<S>,
    pattern: &str,
    path: &str,
) -> Result<(IncludeGlob, String, CompletedRun), BackendError>
where
    S: Sandbox,
{
    let include = IncludeGlob::compile(pattern)?;
    let root = backend.to_confined(path).await?;
    let done = backend.run(&find_argv(&root), FS_TIMEOUT_SECONDS).await?;
    Ok((include, root, done))
}

async fn grep_run<S>(
    backend: &SandboxBackend<S>,
    pattern: &str,
    path: &str,
    glob: Option<&str>,
) -> Result<GrepRun, BackendError>
where
    S: Sandbox,
{
    let include = glob
        .filter(|include| !include.is_empty())
        .map(IncludeGlob::compile)
        .transpose()?;
    let line = Regex::new(GREP_LINE)?;
    let root = backend.to_confined(path).await?;
    let mut argv = vec!["sh", "-c", GREP_SCRIPT, "sh", pattern];
    argv.extend(find_argv(&root));
    let done = backend.run(&argv, FS_TIMEOUT_SECONDS).await?;
    Ok(GrepRun {
        root,
        include,
        line,
        done,
    })
}

fn grep_failure(done: &CompletedRun) -> Option<&str> {
    let stderr = done.stderr.trim();
    // LiteLLM reports xargs' 123 for a search that found nothing as "grep failed"
    let nothing_found = done.exit_code == EXIT_XARGS_BATCH && stderr.is_empty();
    if matches!(done.exit_code, 0_i32 | 1_i32) || nothing_found || !done.stdout.is_empty() {
        return None;
    }
    Some(if stderr.is_empty() {
        "grep failed"
    } else {
        stderr
    })
}

// -P never follows symlinks, so a link to ~/.aws cannot pull host files in
fn find_argv(root: &str) -> Vec<&str> {
    let mut argv = vec!["find", "-P", root, "("];
    for (index, name) in SKIP_DIRS.iter().enumerate() {
        if index > 0 {
            argv.push("-o");
        }
        argv.extend(["-name", name]);
    }
    argv.extend([")", "-prune", "-o", "-type", "f", "-print"]);
    argv
}
