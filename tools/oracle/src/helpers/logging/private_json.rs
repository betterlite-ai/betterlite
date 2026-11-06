use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;
use betterlite_logging::private_json::safe_json::{
    DEFAULT_MAX_RECURSE_DEPTH, safe_dumps, safe_json_loads,
};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::helpers::HelperResult;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Structure {
        data: Value,
        max_depth: Option<usize>,
    },
    Load {
        text: String,
    },
    StageCommit {
        name: String,
        data: Value,
    },
    StageDiscard {
        name: String,
        data: Value,
    },
    Overwrite {
        name: String,
        data: Value,
    },
    OwnerPermissions {
        name: String,
        data: Value,
    },
    SymlinkedParent {
        name: String,
        data: Value,
    },
    EnsurePrivateDir {
        name: String,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Structure { data, max_depth } => structure(data, *max_depth),
            Input::Load { text } => safe_json_loads(text.as_bytes()).map_err(failure),
            Input::StageCommit { .. }
            | Input::StageDiscard { .. }
            | Input::Overwrite { .. }
            | Input::OwnerPermissions { .. }
            | Input::SymlinkedParent { .. }
            | Input::EnsurePrivateDir { .. } => file_operation(input),
        }
    }
}

fn structure(data: &Value, max_depth: Option<usize>) -> HelperResult {
    let dumped =
        safe_dumps(data, max_depth.unwrap_or(DEFAULT_MAX_RECURSE_DEPTH), None).map_err(failure)?;
    serde_json::from_str(&dumped).map_err(failure)
}

fn file_operation(input: &Input) -> HelperResult {
    let root = tempfile::tempdir().map_err(failure)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(failure)?;
    let fixture = Fixture {
        root,
        files: PrivateFiles::default(),
    };
    runtime.block_on(async {
        match input {
            Input::StageCommit { name, data } => fixture.stage(name, data, true).await,
            Input::StageDiscard { name, data } => fixture.stage(name, data, false).await,
            Input::Overwrite { name, data } => fixture.overwrite(name, data).await,
            Input::OwnerPermissions { name, data } => fixture.owner_permissions(name, data).await,
            Input::SymlinkedParent { name, data } => fixture.symlinked_parent(name, data).await,
            Input::EnsurePrivateDir { name } => fixture.ensure_private_dir(name).await,
            Input::Structure { .. } | Input::Load { .. } => Err(failure("not a file operation")),
        }
    })
}

struct Fixture {
    root: TempDir,
    files: PrivateFiles,
}

impl Fixture {
    async fn stage(&self, name: &str, data: &Value, commit: bool) -> HelperResult {
        let target = self.root.path().join(name);
        let staged = self
            .files
            .stage_private_json(&target, data)
            .await
            .map_err(failure)?;
        if commit {
            self.files
                .commit_staged_json(&staged, &target)
                .await
                .map_err(failure)?;
        } else {
            self.files
                .discard_staged_json(&staged)
                .await
                .map_err(failure)?;
        }
        Ok(file_view(&target))
    }

    async fn overwrite(&self, name: &str, data: &Value) -> HelperResult {
        let target = self.root.path().join(name);
        fs::write(&target, "{\"old\": true}").map_err(failure)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).map_err(failure)?;
        self.files
            .overwrite_private_json(&target, data)
            .await
            .map_err(failure)?;
        Ok(file_view(&target))
    }

    async fn owner_permissions(&self, name: &str, data: &Value) -> HelperResult {
        let directory = self.root.path().join("conf");
        self.files
            .ensure_private_dir(&directory)
            .await
            .map_err(failure)?;
        let target = directory.join(name);
        self.files
            .write_private_json(&target, data)
            .await
            .map_err(failure)?;
        let directory_mode = fs::metadata(&directory)
            .map_err(failure)?
            .permissions()
            .mode()
            & 0o777;
        Ok(json!({"directory_mode": directory_mode, "file": file_view(&target)}))
    }

    async fn symlinked_parent(&self, name: &str, data: &Value) -> HelperResult {
        let real = self.root.path().join("real");
        fs::create_dir_all(&real).map_err(failure)?;
        let link = self.root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).map_err(failure)?;
        self.files
            .write_private_json(&link.join(name), data)
            .await
            .map_err(failure)?;
        let parent_is_symlink = fs::symlink_metadata(&link)
            .map_err(failure)?
            .file_type()
            .is_symlink();
        Ok(json!({
            "real": file_view(&real.join(name)),
            "parent_is_symlink": parent_is_symlink,
        }))
    }

    async fn ensure_private_dir(&self, name: &str) -> HelperResult {
        let created = self.root.path().join(name);
        self.files
            .ensure_private_dir(&created)
            .await
            .map_err(failure)?;
        let tightened = self.root.path().join(format!("{name}-tightened"));
        fs::create_dir_all(&tightened).map_err(failure)?;
        fs::set_permissions(&tightened, fs::Permissions::from_mode(0o777)).map_err(failure)?;
        self.files
            .ensure_private_dir(&tightened)
            .await
            .map_err(failure)?;
        let created_mode = fs::metadata(&created)
            .map_err(failure)?
            .permissions()
            .mode()
            & 0o777;
        let tightened_mode = fs::metadata(&tightened)
            .map_err(failure)?
            .permissions()
            .mode()
            & 0o777;
        Ok(json!({"created_mode": created_mode, "tightened_mode": tightened_mode}))
    }
}

fn file_view(target: &Path) -> Value {
    let metadata = fs::metadata(target).ok();
    let file_mode = metadata
        .as_ref()
        .map(|found| found.permissions().mode() & 0o777);
    let content = fs::read(target)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    let leftovers = fs::read_dir(directory).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path() != target)
            .count()
    });
    json!({
        "exists": metadata.is_some(),
        "file_mode": file_mode,
        "content": content,
        "leftovers": leftovers,
    })
}

fn failure<M>(message: M) -> Box<crate::helpers::HelperError>
where
    M: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "PrivateJsonError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
