use super::error::SyntheticEventsError;

pub fn delta_chunks(text: &str, chunk_size: i64) -> Result<Vec<String>, SyntheticEventsError> {
    if chunk_size == 0 {
        return Err(SyntheticEventsError::ZeroChunkSize);
    }
    if chunk_size < 0 {
        return Ok(Vec::new());
    }
    let size = usize::try_from(chunk_size).unwrap_or(usize::MAX);
    let characters: Vec<char> = text.chars().collect();
    Ok(characters
        .chunks(size)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect())
}
