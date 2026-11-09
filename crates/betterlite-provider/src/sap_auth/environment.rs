use std::io;
use std::path::{Path, PathBuf};

pub trait SapEnvironment: Send + Sync {
    fn var(&self, name: &str) -> Option<String>;
    fn file_exists(&self, path: &Path) -> bool;
    fn read_file(&self, path: &Path) -> Result<String, io::Error>;
    fn home_dir(&self) -> PathBuf;
}
