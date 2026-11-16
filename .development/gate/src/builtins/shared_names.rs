use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::{Item, ItemFn, Stmt, Visibility};

use crate::builtins::Context;
use crate::error::GateError;

const ORACLE: &str = "tools/oracle";
// every other oracle crate calls the public functions of these two by name
const SHARED: [&str; 2] = ["tools/oracle/runner", "tools/oracle/helper-api"];

fn rust_files(root: &Path, folder: &str) -> Result<Vec<PathBuf>, GateError> {
    let mut files = Vec::new();
    let mut folders = vec![PathBuf::from(folder)];
    while let Some(current) = folders.pop() {
        let absolute = root.join(&current);
        let listing = fs::read_dir(&absolute).map_err(|source| GateError::Read {
            path: absolute.clone(),
            source,
        })?;
        for entry in listing {
            let entry_name = entry
                .map_err(|source| GateError::Read {
                    path: absolute.clone(),
                    source,
                })?
                .file_name();
            let path = current.join(entry_name);
            let skipped = path
                .file_name()
                .and_then(|text| text.to_str())
                .is_some_and(|text| text.starts_with('.') || text == "target");
            let rust = path.extension().is_some_and(|extension| extension == "rs");
            match (root.join(&path).is_dir(), skipped, rust) {
                (true, false, _) => folders.push(path),
                (false, _, true) => files.push(path),
                (true, true, _) | (false, _, false) => {}
            }
        }
    }
    Ok(files)
}

fn parse(file: &Path) -> Result<Vec<Item>, GateError> {
    let text = fs::read_to_string(file).map_err(|source| GateError::Read {
        path: file.to_owned(),
        source,
    })?;
    let parsed = syn::parse_file(&text).map_err(|source| GateError::RustParse {
        path: file.to_owned(),
        source,
    })?;
    Ok(parsed.items)
}

fn functions<'file>(items: &[&'file Item], found: &mut Vec<&'file ItemFn>) {
    for &item in items {
        if let Item::Fn(function) = item {
            found.push(function);
            let nested: Vec<&Item> = function
                .block
                .stmts
                .iter()
                .filter_map(|statement| match statement {
                    Stmt::Item(nested) => Some(nested),
                    Stmt::Local(_) | Stmt::Expr(..) | Stmt::Macro(_) => None,
                })
                .collect();
            functions(&nested, found);
        }
        if let Item::Mod(module) = item
            && let Some((_, inner)) = &module.content
        {
            let nested: Vec<&Item> = inner.iter().collect();
            functions(&nested, found);
        }
    }
}

pub(crate) fn shared_names(root: &Path) -> Result<BTreeSet<String>, GateError> {
    let mut items = Vec::new();
    for folder in SHARED {
        for file in rust_files(root, folder)? {
            items.extend(parse(&root.join(file))?);
        }
    }
    let top: Vec<&Item> = items.iter().collect();
    let mut found = Vec::new();
    functions(&top, &mut found);
    Ok(found
        .into_iter()
        .filter(|function| matches!(function.vis, Visibility::Public(_)))
        .map(|function| function.sig.ident.to_string())
        .collect())
}

pub(crate) fn name_problems(path: &str, items: &[Item], shared: &BTreeSet<String>) -> Vec<String> {
    let top: Vec<&Item> = items.iter().collect();
    let mut found = Vec::new();
    functions(&top, &mut found);
    found
        .into_iter()
        .map(|function| function.sig.ident.to_string())
        .filter(|name| shared.contains(name))
        .map(|name| {
            format!(
                "{path}: fn {name} has the name of a public function in {}; rename it so a rename of the shared one can't reach it",
                SHARED.join(" or ")
            )
        })
        .collect()
}

pub(crate) fn check(context: &Context<'_>) -> Result<Vec<String>, GateError> {
    let root = &context.development.root;
    let rust: Vec<&String> = context
        .facts
        .changed
        .iter()
        .filter(|path| {
            Path::new(path).starts_with(ORACLE)
                && Path::new(path)
                    .extension()
                    .is_some_and(|extension| extension == "rs")
        })
        .collect();
    if rust.is_empty() {
        return Ok(Vec::new());
    }
    let shared_changed = rust.iter().any(|path| {
        SHARED
            .iter()
            .any(|folder| Path::new(path).starts_with(folder))
    });
    let files: Vec<PathBuf> = if shared_changed {
        rust_files(root, ORACLE)?
    } else {
        rust.iter()
            .map(PathBuf::from)
            .filter(|file| root.join(file).is_file())
            .collect()
    };
    let shared = shared_names(root)?;
    let mut problems = Vec::new();
    for file in files {
        if SHARED.iter().any(|folder| file.starts_with(folder)) {
            continue;
        }
        let items = parse(&root.join(&file))?;
        problems.extend(name_problems(&file.display().to_string(), &items, &shared));
    }
    Ok(problems)
}
