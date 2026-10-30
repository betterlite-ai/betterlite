mod tests;

use std::env::{self, VarError};
use std::fs;
use std::path::{Path, PathBuf};

use camino::Utf8PathBuf;
use determinator::Determinator;
use determinator::rules::DeterminatorRules;
use guppy::CargoMetadata;
use guppy::graph::{DependencyDirection, PackageGraph, PackageMetadata};

use crate::error::GateError;
use crate::facts::resolve_base;
use crate::git::git;
use crate::process::check_command;

pub(crate) const RULES: &str = include_str!("affected/rules.toml");

pub(crate) fn affected(root: &Path) -> Result<Vec<String>, GateError> {
    match env::var("GATE_ALL_TESTS") {
        Ok(all) if !all.is_empty() => return Ok(vec!["--workspace".to_owned()]),
        Ok(_) | Err(VarError::NotPresent) => {}
        Err(VarError::NotUnicode(_)) => return Err(GateError::NotUnicode("GATE_ALL_TESTS")),
    }
    let base = resolve_base(root)?;
    let changed = changed_paths(root, &base)?;
    let snapshot = tempfile::tempdir().map_err(|source| GateError::Write {
        path: env::temp_dir(),
        source,
    })?;
    let old = package_graph(&unpack_base(root, &base, snapshot.path())?)?;
    let new = package_graph(root)?;
    let rules = DeterminatorRules::parse(RULES).map_err(|source| GateError::Affected {
        what: "the affected-package rules".to_owned(),
        source: Box::new(source),
    })?;
    let mut determinator = Determinator::new(&old, &new);
    determinator
        .set_rules(&rules)
        .map_err(|source| GateError::Affected {
            what: "the affected-package rules".to_owned(),
            source: Box::new(source),
        })?;
    determinator.add_changed_paths(&changed);
    let names: Vec<&str> = determinator
        .compute()
        .affected_set
        .packages(DependencyDirection::Forward)
        .filter(PackageMetadata::in_workspace)
        .map(|package| package.name())
        .collect();
    Ok(arguments(new.workspace().iter().len(), names))
}

pub(crate) fn arguments(members: usize, mut names: Vec<&str>) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }
    if names.len() == members {
        return vec!["--workspace".to_owned()];
    }
    names.sort_unstable();
    let flags: Vec<String> = names.iter().map(|name| format!("-p {name}")).collect();
    vec![flags.join(" ")]
}

fn unpack_base(root: &Path, base: &str, scratch: &Path) -> Result<PathBuf, GateError> {
    let archive = scratch.join("base.tar");
    let archive_text = archive.display().to_string();
    git(
        root,
        &["archive", "--format=tar", "-o", &archive_text, base],
    )?;
    let unpacked = scratch.join("base");
    fs::create_dir_all(&unpacked).map_err(|source| GateError::Write {
        path: unpacked.clone(),
        source,
    })?;
    let unpacked_text = unpacked.display().to_string();
    finish("tar", &["-xf", &archive_text, "-C", &unpacked_text], root)?;
    Ok(unpacked)
}

fn changed_paths(root: &Path, base: &str) -> Result<Vec<Utf8PathBuf>, GateError> {
    let tracked = git(root, &["diff", "--name-only", "--no-renames", base])?;
    let untracked = git(root, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(tracked
        .lines()
        .chain(untracked.lines())
        .map(Utf8PathBuf::from)
        .collect())
}

fn package_graph(directory: &Path) -> Result<PackageGraph, GateError> {
    let output = check_command("cargo")
        .args(["metadata", "--format-version", "1", "--locked", "--offline"])
        .current_dir(directory)
        .output()
        .map_err(|source| GateError::Spawn {
            program: "cargo metadata".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(GateError::Failed {
            program: format!("cargo metadata in {}", directory.display()),
            stderr: String::from_utf8(output.stderr)
                .unwrap_or_default()
                .trim()
                .to_owned(),
        });
    }
    let json = String::from_utf8(output.stdout).map_err(|source| GateError::Utf8 {
        what: "cargo metadata".to_owned(),
        source,
    })?;
    CargoMetadata::parse_json(json)
        .and_then(CargoMetadata::build_graph)
        .map_err(|source| GateError::Affected {
            what: format!("the package graph of {}", directory.display()),
            source: Box::new(source),
        })
}

pub(crate) fn finish(program: &str, args: &[&str], directory: &Path) -> Result<(), GateError> {
    let output = check_command(program)
        .args(args)
        .current_dir(directory)
        .output()
        .map_err(|source| GateError::Spawn {
            program: program.to_owned(),
            source,
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(GateError::Failed {
        program: program.to_owned(),
        stderr: String::from_utf8(output.stderr)
            .unwrap_or_default()
            .trim()
            .to_owned(),
    })
}
