use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::iter;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use syn::parse::{Parse, ParseStream, Parser as _};
use syn::punctuated::Punctuated;
use syn::{Ident, Item, LitStr, Token};

use crate::builtins::case_ignores::CaseHead;
use crate::builtins::module_tree::{lexical, owner};
use crate::builtins::shared_names::parse;
use crate::builtins::workspace;
use crate::error::GateError;

const HELPERS: &str = "tools/oracle/src/helpers";
pub(crate) const ROOT: &str = "betterlite-oracle";
pub(crate) const GATEWAY: &str = "gateway";

#[derive(Debug, Deserialize)]
struct HelperCalls {
    call: BTreeMap<String, Value>,
    #[serde(default)]
    calls: Vec<BTreeMap<String, Value>>,
}

struct Operation(LitStr);

impl Parse for Operation {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![=>]>()?;
        input.parse::<Ident>()?;
        Ok(Self(name))
    }
}

pub(crate) fn listed_operations(items: &[Item]) -> syn::Result<Vec<String>> {
    let mut names = Vec::new();
    for item in items {
        let Item::Macro(call) = item else {
            continue;
        };
        if call
            .mac
            .path
            .segments
            .last()
            .is_none_or(|segment| segment.ident != "helper_group")
        {
            continue;
        }
        let listed =
            Punctuated::<Operation, Token![,]>::parse_terminated.parse2(call.mac.tokens.clone())?;
        names.extend(listed.into_iter().map(|operation| operation.0.value()));
    }
    Ok(names)
}

fn operation_packages(root: &Path) -> Result<BTreeMap<String, String>, GateError> {
    let packages = workspace::packages(root)?;
    let folder = lexical(&root.join(HELPERS));
    let listing = fs::read_dir(&folder).map_err(|source| GateError::Read {
        path: folder.clone(),
        source,
    })?;
    let mut owners = BTreeMap::new();
    for entry in listing {
        let path = entry
            .map_err(|source| GateError::Read {
                path: folder.clone(),
                source,
            })?
            .path();
        let file = if path.is_dir() {
            path.join("lib.rs")
        } else {
            path
        };
        let rust = file.extension().is_some_and(|extension| extension == "rs");
        if !rust || !file.is_file() {
            continue;
        }
        let Some(package) = owner(&file, &packages) else {
            continue;
        };
        let items = parse(&file)?;
        let names = listed_operations(&items).map_err(|source| GateError::RustParse {
            path: file.clone(),
            source,
        })?;
        for name in names {
            owners.insert(name, package.name.clone());
        }
    }
    Ok(owners)
}

fn case_files(root: &Path, path: &Path, files: &mut Vec<PathBuf>) -> Result<(), GateError> {
    let absolute = root.join(path);
    if !absolute.is_dir() {
        files.push(path.to_owned());
        return Ok(());
    }
    let listing = fs::read_dir(&absolute).map_err(|source| GateError::Read {
        path: absolute.clone(),
        source,
    })?;
    let mut entries = Vec::new();
    for entry in listing {
        let name = entry
            .map_err(|source| GateError::Read {
                path: absolute.clone(),
                source,
            })?
            .file_name();
        entries.push(path.join(name));
    }
    entries.sort();
    for entry in entries {
        let yaml = entry
            .extension()
            .is_some_and(|extension| extension == "yaml" || extension == "yml");
        if yaml || root.join(&entry).is_dir() {
            case_files(root, &entry, files)?;
        }
    }
    Ok(())
}

// a helper case runs in the group crates that own its operations; the oracle's own groups and every other kind of case run in betterlite-oracle, and only the other kinds need the gateway
pub(crate) fn case_packages(
    path: &Path,
    text: &str,
    owners: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>, GateError> {
    let yaml_error = |source| GateError::Yaml {
        path: path.to_owned(),
        source: Box::new(source),
    };
    let head: CaseHead = serde_saphyr::from_str(text).map_err(yaml_error)?;
    if head.kind.as_deref() != Some("helper") {
        return Ok(BTreeSet::from([GATEWAY.to_owned()]));
    }
    let helper: HelperCalls = serde_saphyr::from_str(text).map_err(yaml_error)?;
    let mut packages = BTreeSet::new();
    for operation in iter::once(&helper.call)
        .chain(&helper.calls)
        .flat_map(BTreeMap::keys)
    {
        let package = owners
            .get(operation)
            .ok_or_else(|| GateError::UnknownOperation {
                path: path.to_owned(),
                operation: operation.clone(),
            })?;
        packages.insert(package.clone());
    }
    if packages.contains(ROOT) {
        return Ok(BTreeSet::from([ROOT.to_owned()]));
    }
    Ok(packages)
}

pub(crate) fn oracle_groups(root: &Path, paths: &[PathBuf]) -> Result<Vec<String>, GateError> {
    let owners = operation_packages(root)?;
    let mut files = Vec::new();
    for path in paths {
        case_files(root, path, &mut files)?;
    }
    let mut lines = Vec::new();
    for file in files {
        let absolute = root.join(&file);
        let text = fs::read_to_string(&absolute).map_err(|source| GateError::Read {
            path: absolute,
            source,
        })?;
        for package in case_packages(&file, &text, &owners)? {
            lines.push(format!("{package} {}", file.display()));
        }
    }
    Ok(lines)
}
