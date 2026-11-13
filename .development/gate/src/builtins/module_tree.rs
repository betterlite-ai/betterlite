use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use syn::{Attribute, Expr, ExprLit, Item, ItemMacro, ItemMod, Lit, LitStr, Meta};

use crate::builtins::Context;
use crate::builtins::workspace::{self, Package};
use crate::error::GateError;

#[derive(Default)]
pub(crate) struct Walk {
    reached: BTreeSet<PathBuf>,
    // an item macro such as helper_group! can declare any module file in these folders
    macro_folders: BTreeSet<PathBuf>,
}

struct Folders<'walk> {
    file: &'walk Path,
    folder: PathBuf,
    path_base: PathBuf,
    children: PathBuf,
}

fn lexical(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                clean.push(component);
            }
        }
    }
    clean
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

fn path_attribute(attributes: &[Attribute]) -> Option<String> {
    attributes.iter().find_map(|attribute| {
        if let Meta::NameValue(pair) = &attribute.meta
            && pair.path.is_ident("path")
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(text),
                ..
            }) = &pair.value
        {
            Some(text.value())
        } else {
            None
        }
    })
}

impl Walk {
    pub(crate) fn from_targets(targets: &[&Path]) -> Result<Self, GateError> {
        let mut walk = Self::default();
        for target in targets {
            walk.module_file(target, true)?;
        }
        Ok(walk)
    }

    fn module_file(&mut self, declared: &Path, owns_folder: bool) -> Result<(), GateError> {
        let file = lexical(declared);
        if !file.is_file() || !self.reached.insert(file.clone()) {
            return Ok(());
        }
        let folder = file.parent().map(Path::to_path_buf).unwrap_or_default();
        let children = if owns_folder {
            folder.clone()
        } else {
            folder.join(file.file_stem().unwrap_or_default())
        };
        let items = parse(&file)?;
        let folders = Folders {
            file: &file,
            folder: folder.clone(),
            path_base: folder,
            children,
        };
        self.items(&items, &folders)
    }

    fn items(&mut self, items: &[Item], folders: &Folders<'_>) -> Result<(), GateError> {
        for item in items {
            if let Item::Mod(module) = item {
                self.module(module, folders)?;
            }
            if let Item::Macro(call) = item {
                self.item_macro(call, folders)?;
            }
        }
        Ok(())
    }

    fn module(&mut self, module: &ItemMod, folders: &Folders<'_>) -> Result<(), GateError> {
        let name = module.ident.to_string();
        let path = path_attribute(&module.attrs);
        if let Some((_, inner)) = &module.content {
            let nested = path.map_or_else(
                || folders.children.join(&name),
                |relative| folders.path_base.join(relative),
            );
            let inline = Folders {
                file: folders.file,
                folder: folders.folder.clone(),
                path_base: nested.clone(),
                children: nested,
            };
            return self.items(inner, &inline);
        }
        if let Some(relative) = path {
            return self.module_file(&folders.path_base.join(relative), true);
        }
        self.module_file(&folders.children.join(format!("{name}.rs")), false)?;
        self.module_file(&folders.children.join(&name).join("mod.rs"), true)
    }

    fn item_macro(&mut self, call: &ItemMacro, folders: &Folders<'_>) -> Result<(), GateError> {
        if call.ident.is_some() {
            return Ok(());
        }
        if !call.mac.path.is_ident("include") {
            self.macro_folders.insert(lexical(&folders.children));
            return Ok(());
        }
        let target: LitStr = call
            .mac
            .parse_body()
            .map_err(|source| GateError::RustParse {
                path: folders.file.to_owned(),
                source,
            })?;
        let included = lexical(&folders.folder.join(target.value()));
        if !included.is_file() || !self.reached.insert(included.clone()) {
            return Ok(());
        }
        let included_folder = included.parent().map(Path::to_path_buf).unwrap_or_default();
        let spliced = Folders {
            file: &included,
            folder: included_folder.clone(),
            path_base: included_folder,
            children: folders.children.clone(),
        };
        self.items(&parse(&included)?, &spliced)
    }

    pub(crate) fn reaches(&self, file: &Path) -> bool {
        self.reached.contains(file)
            || self
                .macro_folders
                .iter()
                .any(|folder| file.starts_with(folder))
    }
}

fn owner<'packages>(file: &Path, packages: &'packages [Package]) -> Option<&'packages Package> {
    let (package, folder) = packages
        .iter()
        .filter_map(|package| Some((package, lexical(package.manifest_path.parent()?))))
        .filter(|(_, folder)| file.starts_with(folder))
        .max_by_key(|(_, folder)| folder.components().count())?;
    // a nested manifest that is no workspace member, such as an excluded fuzz crate, owns the file instead
    let nested = file
        .ancestors()
        .skip(1)
        .take_while(|ancestor| *ancestor != folder)
        .any(|ancestor| ancestor.join("Cargo.toml").is_file());
    (!nested).then_some(package)
}

fn unreached(package: &Package, owned: &[&(&String, PathBuf)]) -> Result<Vec<String>, GateError> {
    let targets: Vec<&Path> = package
        .targets
        .iter()
        .map(|target| target.src_path.as_path())
        .collect();
    let walk = Walk::from_targets(&targets)?;
    Ok(owned
        .iter()
        .filter(|(_, file)| !walk.reaches(file))
        .map(|(path, _)| {
            format!(
                "{path} never compiles: no mod line, #[path] or include! reaches it from a target of {}; declare it in its parent module",
                package.name
            )
        })
        .collect())
}

pub(crate) fn check(context: &Context<'_>) -> Result<Vec<String>, GateError> {
    let root = &context.development.root;
    let changed: Vec<(&String, PathBuf)> = context
        .facts
        .changed
        .iter()
        .filter(|path| {
            Path::new(path)
                .extension()
                .is_some_and(|extension| extension == "rs")
        })
        .map(|path| (path, lexical(&root.join(path))))
        .filter(|(_, file)| file.is_file())
        .collect();
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    let packages = workspace::packages(root)?;
    let mut problems = Vec::new();
    for package in &packages {
        let owned: Vec<&(&String, PathBuf)> = changed
            .iter()
            .filter(|(_, file)| {
                owner(file, &packages).is_some_and(|found| found.name == package.name)
            })
            .collect();
        if !owned.is_empty() {
            problems.extend(unreached(package, &owned)?);
        }
    }
    Ok(problems)
}
