use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write as _};
use std::iter;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use betterlite_oracle_helper_api::group::Group;

use crate::case::helper::HelperCase;
use crate::compare;
use crate::compare::helper::HelperOutput;
use crate::error::HelperRunError;
use answers::Answers;
use betterlite_oracle_helper_api::error::HelperError;
use python::{process_environment, python_stdout};

// tools/oracle, where the cases, the fuzz corpus, the fixtures and the Python helpers live
pub const ORACLE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(try_from = "BTreeMap<String, serde_json::Value>")]
pub struct HelperCall {
    pub operation: String,
    pub input: serde_json::Value,
}

impl TryFrom<BTreeMap<String, serde_json::Value>> for HelperCall {
    type Error = String;

    fn try_from(call: BTreeMap<String, serde_json::Value>) -> Result<Self, Self::Error> {
        let mut entries = call.into_iter();
        match (entries.next(), entries.next()) {
            (Some((operation, input)), None) => Ok(Self { operation, input }),
            _ => Err("a helper call names exactly one operation".to_owned()),
        }
    }
}

impl HelperCall {
    // None when none of the groups lists the operation, so a crate built with some groups can leave the other calls alone
    #[must_use]
    pub fn owned_check(&self, groups: &[Group]) -> Option<Result<(), String>> {
        groups
            .iter()
            .find_map(|group| (group.check)(&self.operation, &self.input))
    }

    // a group crate sees only its own operations, so the oracle checks every call against all groups when it loads a case
    pub fn check(&self, groups: &[Group]) -> Result<(), String> {
        self.owned_check(groups).unwrap_or_else(|| {
            Err(format!(
                "unknown variant `{}`, expected a registered helper operation",
                self.operation
            ))
        })
    }
}

pub enum HelperRun {
    Match,
    Difference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperLeft {
    Native,
    Python,
}

#[must_use]
pub const fn left_side(self_check: bool) -> HelperLeft {
    if self_check {
        HelperLeft::Python
    } else {
        HelperLeft::Native
    }
}

pub fn run_case(
    groups: &[Group],
    case: &HelperCase,
    self_check: bool,
) -> Result<HelperRun, HelperRunError> {
    let total = case.calls.len().saturating_add(1);
    for (index, call) in iter::once(&case.call).chain(case.calls.iter()).enumerate() {
        let label = if total > 1 {
            format!("{} call {index}", case.name)
        } else {
            case.name.clone()
        };
        let (left, right) = run_pair(groups, call, &case.env, self_check)?;
        let Some(difference) = compare::compare_helper(&left, &right, &case.compare.ignore) else {
            continue;
        };
        let pointer = difference.pointer;
        let (left_name, right_name) = match left_side(self_check) {
            HelperLeft::Native => ("betterlite", "litellm"),
            HelperLeft::Python => ("first litellm run", "second litellm run"),
        };
        let left_value = value_at(&left, &pointer);
        let right_value = value_at(&right, &pointer);
        writeln!(
            io::stderr().lock(),
            "FAIL {label}: {pointer} ({})\n     {left_name}: {left_value}\n     {right_name}: {right_value}",
            difference.message
        )
        .map_err(HelperRunError::Output)?;
        return Ok(HelperRun::Difference);
    }
    writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(HelperRunError::Output)?;
    Ok(HelperRun::Match)
}

pub fn run_helper_cases(
    groups: &[Group],
    cases: &[HelperCase],
    self_check: bool,
) -> Result<HelperRun, HelperRunError> {
    let mut outcome = HelperRun::Match;
    for case in cases {
        if matches!(run_case(groups, case, self_check)?, HelperRun::Difference) {
            outcome = HelperRun::Difference;
        }
    }
    Ok(outcome)
}

fn value_at(output: &HelperOutput, pointer: &str) -> String {
    serde_json::to_value(output)
        .ok()
        .and_then(|value| value.pointer(pointer).map(ToString::to_string))
        .map_or_else(
            || "absent".to_owned(),
            |text| text.chars().take(400).collect(),
        )
}

pub fn run_pair(
    groups: &[Group],
    call: &HelperCall,
    env: &BTreeMap<String, String>,
    self_check: bool,
) -> Result<(HelperOutput, HelperOutput), HelperRunError> {
    let root = Path::new(ORACLE_DIR).join("../..");
    Ok(match left_side(self_check) {
        HelperLeft::Native => {
            let answers = Answers::from_env(&root)?;
            (invoke(groups, call), answers.answer(&root, call, env)?)
        }
        HelperLeft::Python => {
            sync_litellm(&root)?;
            let process_env = process_environment();
            let [first, second] = [(), ()].map(|()| {
                let stdout_output = python_stdout(&root, call, env, &process_env)?;
                serde_json::from_slice::<HelperOutput>(&stdout_output)
                    .map_err(HelperRunError::HelperJson)
            });
            (first?, second?)
        }
    })
}

#[must_use]
pub fn invoke(groups: &[Group], call: &HelperCall) -> HelperOutput {
    let result = groups
        .iter()
        .find_map(|group| (group.invoke)(&call.operation, &call.input))
        .unwrap_or_else(|| {
            let message = format!("no helper operation is named {}", call.operation);
            Err(Box::new(HelperError {
                class: "UnknownOperation".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            }))
        });
    match result {
        Ok(value) => HelperOutput {
            ok: true,
            // the Python side reads a null value as absent, so a Rust null goes out the same way
            value: (!value.is_null()).then_some(value),
            error: None,
            warnings: Vec::new(),
            effects: Vec::new(),
        },
        Err(failure) => {
            let mut error = *failure;
            error.error_type.get_or_insert(None);
            error.param.get_or_insert(None);
            error.code.get_or_insert(None);
            HelperOutput {
                ok: false,
                value: None,
                error: Some(error),
                warnings: Vec::new(),
                effects: Vec::new(),
            }
        }
    }
}

pub fn sync_litellm(root: &Path) -> Result<(), HelperRunError> {
    let project = root.join("tools/oracle/litellm");
    let status = Command::new("uv")
        .args(["sync", "--locked"])
        .current_dir(project)
        .status()
        .map_err(HelperRunError::ProcessSync)?;
    if status.success() {
        Ok(())
    } else {
        Err(HelperRunError::ProcessSyncExited(status))
    }
}

// Prisma writes client.py only when generate runs, so its presence is the record that the client exists
pub fn generate_prisma_client(root: &Path) -> Result<(), HelperRunError> {
    let project = root.join("tools/oracle/litellm");
    let site_packages = venv_site_packages(&project)?;
    if site_packages.join("prisma/client.py").is_file() {
        return Ok(());
    }
    // the booted tests run as separate processes against one Prisma CLI cache, so a second process waits here for the first
    let lock_path = env::temp_dir().join("betterlite-oracle-prisma-generate.lock");
    let _lock = lock_prisma_generate(&lock_path)?;
    if site_packages.join("prisma/client.py").is_file() {
        return Ok(());
    }
    let schema = site_packages.join("litellm/proxy/schema.prisma");
    if !schema.is_file() {
        return Err(HelperRunError::PrismaMissing { path: schema });
    }
    let status = Command::new("uv")
        .args(["run", "--locked", "prisma", "generate", "--schema"])
        .arg(&schema)
        .current_dir(&project)
        .status()
        .map_err(HelperRunError::PrismaGenerate)?;
    if status.success() {
        Ok(())
    } else {
        Err(HelperRunError::PrismaGenerateExited(status))
    }
}

// prisma installs its CLI into one cache under the user's home, and concurrent npm installs there corrupt it
fn lock_prisma_generate(path: &Path) -> Result<fs::File, HelperRunError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| HelperRunError::PrismaLock {
            path: path.to_path_buf(),
            source,
        })?;
    file.lock().map_err(|source| HelperRunError::PrismaLock {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(file)
}

// LiteLLM's key lookup reads a key's models as a list and rejects NULL, and the fixture's seed leaves that column empty
const KEY_MODELS_SQL: &str =
    "UPDATE \"LiteLLM_VerificationToken\" SET models = ARRAY[]::TEXT[] WHERE models IS NULL";

pub fn fill_key_models(root: &Path, database_url: &str) -> Result<(), HelperRunError> {
    let project = root.join("tools/oracle/litellm");
    let schema = venv_site_packages(&project)?.join("litellm/proxy/schema.prisma");
    if !schema.is_file() {
        return Err(HelperRunError::PrismaMissing { path: schema });
    }
    let mut child = Command::new("uv")
        .args([
            "run", "--locked", "prisma", "db", "execute", "--stdin", "--schema",
        ])
        .arg(&schema)
        .env("DATABASE_URL", database_url)
        .current_dir(&project)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(HelperRunError::PrismaExecute)?;
    let mut stdin = child.stdin.take().ok_or(HelperRunError::PrismaStdin)?;
    let written = stdin
        .write_all(KEY_MODELS_SQL.as_bytes())
        .map_err(HelperRunError::PrismaExecute);
    drop(stdin);
    let status = child.wait().map_err(HelperRunError::PrismaExecute)?;
    written?;
    if status.success() {
        Ok(())
    } else {
        Err(HelperRunError::PrismaExecuteExited(status))
    }
}

fn venv_site_packages(project: &Path) -> Result<PathBuf, HelperRunError> {
    let library = project.join(".venv/lib");
    let Some(python) = python_folder(&library) else {
        return Err(HelperRunError::PrismaMissing { path: library });
    };
    Ok(python.join("site-packages"))
}

fn python_folder(library: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(library).ok()?;
    let mut folders = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("python3."))
        });
    let folder = folders.next()?;
    folders.next().is_none().then_some(folder)
}

mod answers;
mod python;
mod tests;
