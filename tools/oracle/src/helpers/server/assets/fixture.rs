use std::io;
use std::os::unix::fs::symlink;
use std::path::Path;

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Entry {
    Dir {
        path: String,
    },
    File {
        path: String,
        #[serde(default)]
        content_hex: String,
    },
    Symlink {
        path: String,
        target: String,
    },
}

pub(super) fn materialize(root: &Path, entries: &[Entry]) -> io::Result<()> {
    for entry in entries {
        match entry {
            Entry::Dir { path } => {
                std::fs::create_dir_all(root.join(path))?;
            }
            Entry::File { path, content_hex } => {
                let full = root.join(path);
                create_parent(&full)?;
                let bytes = hex::decode(content_hex)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                std::fs::write(full, bytes)?;
            }
            Entry::Symlink { path, target } => {
                let full = root.join(path);
                create_parent(&full)?;
                symlink(target, full)?;
            }
        }
    }
    Ok(())
}

fn create_parent(path: &Path) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)
}

pub(super) fn relative_to(root: &Path, target: &Path) -> Option<String> {
    let suffix = target.strip_prefix(root).ok()?;
    if suffix.as_os_str().is_empty() {
        Some(".".to_owned())
    } else {
        Some(suffix.to_string_lossy().into_owned())
    }
}
