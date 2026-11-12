use rust_decimal::Decimal;

use super::content::{file_bytes, read_whole};
use super::decoder::AudioDecoder;
use super::{AudioFile, AuthorizedAudioFiles, TupleContent};

#[must_use]
pub fn calculate_request_duration(
    file: &mut AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
    decoder: AudioDecoder<'_>,
) -> Option<Decimal> {
    let AudioDecoder::Available(available) = decoder else {
        return None;
    };
    let content = duration_content(file, files)?;
    available.duration(&content)
}

fn duration_content(file: &mut AudioFile<'_>, files: &dyn AuthorizedAudioFiles) -> Option<Vec<u8>> {
    let AudioFile::Tuple(tuple) = file else {
        return file_bytes(file, files);
    };
    match &mut tuple.content {
        Some(TupleContent::Bytes(content)) => Some(content.to_vec()),
        Some(TupleContent::Stream(stream)) => read_whole(&mut **stream),
        _ => None,
    }
}
