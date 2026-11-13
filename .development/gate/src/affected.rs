mod tests;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use camino::Utf8PathBuf;
use clap::Args;
use determinator::Determinator;
use determinator::rules::DeterminatorRules;
use guppy::CargoMetadata;
use guppy::graph::{DependencyDirection, PackageGraph, PackageMetadata};

use crate::error::GateError;
use crate::facts::resolve_base;
use crate::git::git;
use crate::process::check_command;

pub(crate) const RULES: &str = include_str!("affected/rules.toml");
// .github/scripts/test-seconds.sh writes it from a main run, and a package it lacks counts as one second
const SECONDS: &str = ".config/test-seconds.toml";

#[derive(Clone, Copy, Args)]
pub(crate) struct Shard {
    #[arg(
        long = "shard",
        requires = "count",
        value_name = "I",
        help = "Print only shard I: the packages split over the --shards count by the test seconds in .config/test-seconds.toml, largest first onto the lightest shard"
    )]
    pub(crate) index: Option<u16>,
    #[arg(
        long = "shards",
        requires = "index",
        value_name = "N",
        help = "How many shards --shard splits the packages over"
    )]
    pub(crate) count: Option<u16>,
}

pub(crate) fn affected(
    root: &Path,
    only_edited: bool,
    shard: Shard,
) -> Result<Vec<String>, GateError> {
    let split = shard.index.zip(shard.count);
    if let Some((index, count)) = split.filter(|&(index, count)| index == 0 || index > count) {
        return Err(GateError::Shard { index, count });
    }
    let all = env::var_os("GATE_ALL_TESTS").is_some_and(|value| !value.is_empty());
    if all && split.is_none() {
        return Ok(vec!["--workspace".to_owned()]);
    }
    let graph = package_graph(root)?;
    let workspace = graph.workspace();
    let names: Vec<String> = if all {
        workspace
            .iter()
            .map(|package| package.name().to_owned())
            .collect()
    } else {
        packages(root, &resolve_base(root)?, &graph, only_edited)?
    };
    let Some((index, count)) = split else {
        return Ok(arguments(
            workspace.iter().len(),
            names.iter().map(String::as_str).collect(),
        ));
    };
    let path = root.join(SECONDS);
    let text = fs::read_to_string(&path).map_err(|source| GateError::Read {
        path: path.clone(),
        source,
    })?;
    let seconds: BTreeMap<String, u64> =
        toml::from_str(&text).map_err(|source| GateError::Toml {
            path,
            source: Box::new(source),
        })?;
    Ok(arguments(
        workspace.iter().len(),
        share(&names, &seconds, usize::from(index), usize::from(count)),
    ))
}

fn packages(
    root: &Path,
    base: &str,
    new: &PackageGraph,
    only_edited: bool,
) -> Result<Vec<String>, GateError> {
    let changed = changed_paths(root, base)?;
    let snapshot = tempfile::tempdir().map_err(|source| GateError::Write {
        path: env::temp_dir(),
        source,
    })?;
    let old = package_graph(&unpack_base(root, base, snapshot.path())?)?;
    let rules = DeterminatorRules::parse(RULES).map_err(|source| GateError::Affected {
        what: "the affected-package rules".to_owned(),
        source: Box::new(source),
    })?;
    let mut determinator = Determinator::new(&old, new);
    determinator
        .set_rules(&rules)
        .map_err(|source| GateError::Affected {
            what: "the affected-package rules".to_owned(),
            source: Box::new(source),
        })?;
    determinator.add_changed_paths(&changed);
    let computed = determinator.compute();
    let selected = if only_edited {
        computed
            .path_changed_set
            .union(&computed.summary_changed_set)
    } else {
        computed.affected_set
    };
    Ok(selected
        .packages(DependencyDirection::Forward)
        .filter(PackageMetadata::in_workspace)
        .map(|package| package.name().to_owned())
        .collect())
}

// each shard computes the split on its own, so ties go by name
pub(crate) fn share<'names>(
    names: &'names [String],
    seconds: &BTreeMap<String, u64>,
    index: usize,
    count: usize,
) -> Vec<&'names str> {
    let mut weighted: Vec<(u64, &str)> = names
        .iter()
        .map(|name| {
            (
                seconds.get(name).copied().unwrap_or(1).max(1),
                name.as_str(),
            )
        })
        .collect();
    weighted.sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    let mut loads = vec![0_u64; count];
    let mut mine = Vec::new();
    for (weight, name) in weighted {
        let Some((bin, load)) = loads.iter_mut().enumerate().min_by_key(|entry| *entry.1) else {
            break;
        };
        *load = load.saturating_add(weight);
        if bin.saturating_add(1) == index {
            mine.push(name);
        }
    }
    mine
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
