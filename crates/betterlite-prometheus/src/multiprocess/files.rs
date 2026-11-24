use std::str;

use super::MultiprocessError;

mod tests;

// the file starts with the used length and four zero bytes, and every entry is a length, its key and two doubles
const USED_OFFSET: usize = 0;
const HEADER_BYTES: usize = 8;
const LENGTH_BYTES: usize = 4;
const DOUBLE_BYTES: usize = 8;
const TWO_DOUBLES_BYTES: usize = 16;
const ALIGNMENT_BYTES: usize = 8;
// prometheus_client's mmap_dict._INITIAL_MMAP_SIZE: a fresh file is one 64 KiB page of zero bytes
const INITIAL_CAPACITY: usize = 0x0001_0000;

#[derive(Clone, Debug, PartialEq)]
pub struct DbEntry {
    pub key: String,
    pub value: f64,
    pub timestamp: f64,
}

// prometheus_client's mmap_dict.read_all_values_from_file and _read_all_values: the entries between the header and the used length
pub fn decode_db_file(bytes: &[u8]) -> Result<Vec<DbEntry>, MultiprocessError> {
    let Some(header) = bytes
        .get(USED_OFFSET..)
        .and_then(<[u8]>::first_chunk::<LENGTH_BYTES>)
    else {
        return Err(MultiprocessError::Truncated {
            offset: USED_OFFSET,
        });
    };
    let used_field = i32::from_le_bytes(*header);
    let Ok(used) = usize::try_from(used_field) else {
        return Ok(Vec::new());
    };
    if used <= HEADER_BYTES {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    let mut position = HEADER_BYTES;
    while position < used {
        let (entry, next) = decode_entry(bytes, position, used)?;
        entries.push(entry);
        position = next;
    }
    Ok(entries)
}

// one entry: its length, the padded key and the value with its timestamp
fn decode_entry(
    bytes: &[u8],
    position: usize,
    used: usize,
) -> Result<(DbEntry, usize), MultiprocessError> {
    let Some(length_bytes) = bytes
        .get(position..)
        .and_then(<[u8]>::first_chunk::<LENGTH_BYTES>)
    else {
        return Err(MultiprocessError::Truncated { offset: position });
    };
    let Ok(key_length) = usize::try_from(i32::from_le_bytes(*length_bytes)) else {
        return Err(MultiprocessError::Corrupted);
    };
    let Some(key_end) = position.checked_add(key_length) else {
        return Err(MultiprocessError::Corrupted);
    };
    if key_end > used {
        return Err(MultiprocessError::Corrupted);
    }
    let key_start = position.saturating_add(LENGTH_BYTES);
    let key_byte_end = key_start.saturating_add(key_length);
    let Some(key_bytes) = bytes.get(key_start..key_byte_end) else {
        return Err(MultiprocessError::Truncated { offset: key_start });
    };
    let key = str::from_utf8(key_bytes)
        .map_err(|_utf8| MultiprocessError::NonUtf8Key { offset: key_start })?
        .to_owned();
    let value_start = key_byte_end.saturating_add(entry_padding(key_length));
    let (value, timestamp) = decode_doubles(bytes, value_start)?;
    let entry = DbEntry {
        key,
        value,
        timestamp,
    };
    Ok((entry, value_start.saturating_add(TWO_DOUBLES_BYTES)))
}

fn decode_doubles(bytes: &[u8], start: usize) -> Result<(f64, f64), MultiprocessError> {
    let Some(value_bytes) = bytes
        .get(start..)
        .and_then(<[u8]>::first_chunk::<DOUBLE_BYTES>)
    else {
        return Err(MultiprocessError::Truncated { offset: start });
    };
    let timestamp_start = start.saturating_add(DOUBLE_BYTES);
    let Some(timestamp_bytes) = bytes
        .get(timestamp_start..)
        .and_then(<[u8]>::first_chunk::<DOUBLE_BYTES>)
    else {
        return Err(MultiprocessError::Truncated {
            offset: timestamp_start,
        });
    };
    Ok((
        f64::from_le_bytes(*value_bytes),
        f64::from_le_bytes(*timestamp_bytes),
    ))
}

// prometheus_client's mmap_dict writer pads the key with 8 - (len + 4) % 8 spaces, a full 8 when 4 + len is aligned
const fn entry_padding(key_length: usize) -> usize {
    let head = key_length.saturating_add(LENGTH_BYTES);
    match head.checked_rem(ALIGNMENT_BYTES) {
        Some(remainder) => ALIGNMENT_BYTES.saturating_sub(remainder),
        None => ALIGNMENT_BYTES,
    }
}

// the bytes the mmap_dict writer's _init_value and write_value leave for one key
pub(crate) fn encode_entry(key: &str, value: f64, timestamp: f64) -> Vec<u8> {
    let key_length = key.len();
    let length = i32::try_from(key_length).unwrap_or(i32::MAX);
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&length.to_le_bytes());
    encoded.extend_from_slice(key.as_bytes());
    let padded_end = encoded.len().saturating_add(entry_padding(key_length));
    encoded.resize(padded_end, b' ');
    encoded.extend_from_slice(&value.to_le_bytes());
    encoded.extend_from_slice(&timestamp.to_le_bytes());
    encoded
}

// the whole file those writes leave: the used length, the entries, then zeros up to the next power of two from _INITIAL_MMAP_SIZE up
#[must_use]
pub fn encode_db_file(entries: &[DbEntry]) -> Vec<u8> {
    let mut body = Vec::new();
    for entry in entries {
        body.extend_from_slice(&encode_entry(&entry.key, entry.value, entry.timestamp));
    }
    let used = body.len().saturating_add(HEADER_BYTES);
    let length = i32::try_from(used).unwrap_or(i32::MAX);
    let mut file = Vec::new();
    file.extend_from_slice(&length.to_le_bytes());
    file.extend_from_slice(&[0_u8; 4]);
    file.extend_from_slice(&body);
    file.resize(used.max(INITIAL_CAPACITY).next_power_of_two(), 0);
    file
}
