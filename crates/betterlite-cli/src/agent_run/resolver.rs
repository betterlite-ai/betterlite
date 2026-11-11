use std::ffi::OsString;
use std::path::PathBuf;
pub trait BinaryResolver {
    fn resolve(&self, command: &str) -> Option<String>;
}
pub struct PathResolver {
    pub search_path: OsString,
    pub current_dir: PathBuf,
}
impl BinaryResolver for PathResolver {
    fn resolve(&self, command: &str) -> Option<String> {
        let found = which::which_in(command, Some(&self.search_path), &self.current_dir).ok()?;
        found.to_str().map(str::to_owned)
    }
}
