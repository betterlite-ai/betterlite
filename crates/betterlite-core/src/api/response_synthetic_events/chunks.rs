use super::error::SyntheticEventsError;

pub fn delta_chunks(text: &str, chunk_size: i64) -> Result<Vec<String>, SyntheticEventsError> {
    if chunk_size == 0 {
        return Err(SyntheticEventsError::ZeroChunkSize);
    }
    let Ok(size) = usize::try_from(chunk_size) else {
        return Ok(Vec::new());
    };
    let characters: Vec<char> = text.chars().collect();
    Ok(characters
        .chunks(size)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect())
}
