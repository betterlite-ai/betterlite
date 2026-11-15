use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;

use crate::transcription::params::Params;

pub(super) struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
}

pub(super) fn params(source: &str) -> Params {
    serde_json::from_str(source).unwrap()
}

pub(super) fn env(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
