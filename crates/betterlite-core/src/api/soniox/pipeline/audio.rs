use betterlite_openai::api::audio_file_input::{AudioFile, AudioTuple, TupleContent};

// process_audio_file takes the file by value; the reborrow keeps it available for the post-call log
pub(super) fn reborrow<'file>(file: &'file mut AudioFile<'_>) -> AudioFile<'file> {
    match file {
        AudioFile::Bytes(content) => AudioFile::Bytes(content),
        AudioFile::ByteArray(content) => AudioFile::ByteArray(content),
        AudioFile::Stream(stream) => AudioFile::Stream(&mut **stream),
        AudioFile::Tuple(tuple) => AudioFile::Tuple(AudioTuple {
            filename: tuple.filename,
            content: tuple.content.as_mut().map(reborrow_content),
        }),
        AudioFile::Path(path) => AudioFile::Path(path),
        AudioFile::Text(text) => AudioFile::Text(text),
        AudioFile::Unsupported { python_class } => AudioFile::Unsupported { python_class },
    }
}

fn reborrow_content<'content>(content: &'content mut TupleContent<'_>) -> TupleContent<'content> {
    match content {
        TupleContent::Bytes(bytes) => TupleContent::Bytes(bytes),
        TupleContent::ByteArray(bytes) => TupleContent::ByteArray(bytes),
        TupleContent::Text(text) => TupleContent::Text(text),
        TupleContent::Path(path) => TupleContent::Path(path),
        TupleContent::Stream(stream) => TupleContent::Stream(&mut **stream),
        TupleContent::Unsupported { python_class } => TupleContent::Unsupported { python_class },
    }
}
