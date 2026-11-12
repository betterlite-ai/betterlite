use super::{AudioFile, AudioStream, AuthorizedAudioFiles};

pub(super) fn read_whole(stream: &mut dyn AudioStream) -> Option<Vec<u8>> {
    let position = match stream.tell() {
        Some(position) => Some(position.ok()?),
        None => None,
    };
    if let Some(rewound) = stream.seek(0) {
        rewound.ok()?;
    }
    let content = stream.read().ok()?;
    if let Some(restore_position) = position
        && let Some(restored) = stream.seek(restore_position)
    {
        restored.ok()?;
    }
    content
}

pub(super) fn file_bytes(
    file: &mut AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> Option<Vec<u8>> {
    match file {
        AudioFile::Bytes(content) | AudioFile::ByteArray(content) => Some(content.to_vec()),
        AudioFile::Path(path) => files.read(path).ok(),
        AudioFile::Stream(stream) => read_whole(&mut **stream),
        AudioFile::Text(_) | AudioFile::Tuple(_) | AudioFile::Unsupported { .. } => None,
    }
}
