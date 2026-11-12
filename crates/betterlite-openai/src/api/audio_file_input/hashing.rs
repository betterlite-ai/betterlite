use sha2::{Digest as _, Sha256};

use super::content::{file_bytes, read_whole};
use super::{AudioFile, AuthorizedAudioFiles, TupleContent, get_audio_file_name};

#[must_use]
pub fn get_audio_file_content_hash(
    file: &mut AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> String {
    let (fallback, content) = hash_parts(file, files);
    if let Some(bytes) = content {
        return hex::encode(Sha256::digest(bytes));
    }
    let name = if fallback.is_empty() {
        get_audio_file_name(file)
    } else {
        fallback
    };
    hex::encode(Sha256::digest(name.as_bytes()))
}

pub(super) fn hash_parts(
    file: &mut AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> (String, Option<Vec<u8>>) {
    let AudioFile::Tuple(tuple) = file else {
        return (get_audio_file_name(file), file_bytes(file, files));
    };
    let fallback = tuple.filename.to_owned();
    let Some(content) = &mut tuple.content else {
        return (fallback, None);
    };
    let (overridden, bytes) = hash_content(content, files);
    (overridden.unwrap_or(fallback), bytes)
}

fn hash_content(
    content: &mut TupleContent<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> (Option<String>, Option<Vec<u8>>) {
    match content {
        TupleContent::Bytes(bytes) | TupleContent::ByteArray(bytes) => (None, Some(bytes.to_vec())),
        TupleContent::Text(text) => (Some((*text).to_owned()), None),
        TupleContent::Path(path) => files.read(path).map_or_else(
            |_| (Some(path.to_str().unwrap_or_default().to_owned()), None),
            |bytes| (None, Some(bytes)),
        ),
        TupleContent::Stream(stream) => (None, read_whole(&mut **stream)),
        TupleContent::Unsupported { .. } => (None, None),
    }
}
