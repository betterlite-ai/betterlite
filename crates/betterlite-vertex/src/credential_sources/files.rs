use std::fs;
use std::io;
use std::path::Path;

// the credentials and the gcloud configuration are read through this seam so a fixture stands in for the disk
pub trait FileReader: Send + Sync {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
}

pub struct StdFiles;

impl FileReader for StdFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}
