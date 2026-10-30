use std::collections::BTreeSet;

use super::{CatalogError, MAX_CATALOG_DEPTH};

pub(super) fn check_duplicates(bytes: &[u8]) -> Result<(), CatalogError> {
    let mut cursor = 0;
    scan_value(bytes, &mut cursor, 0, String::new())?;
    Ok(())
}

fn scan_value(
    bytes: &[u8],
    cursor: &mut usize,
    depth: usize,
    path: String,
) -> Result<(), CatalogError> {
    if depth >= MAX_CATALOG_DEPTH {
        return Err(scan_error("model catalog nesting exceeds limit"));
    }
    skip_space(bytes, cursor);
    let byte = bytes
        .get(*cursor)
        .copied()
        .ok_or_else(|| scan_error("unexpected end"))?;
    match byte {
        b'{' => scan_object(bytes, cursor, depth, path),
        b'[' => scan_array(bytes, cursor, depth, &path),
        b'"' => {
            scan_string(bytes, cursor)?;
            Ok(())
        }
        _ => {
            scan_scalar(bytes, cursor);
            Ok(())
        }
    }
}

fn scan_object(
    bytes: &[u8],
    cursor: &mut usize,
    depth: usize,
    path: String,
) -> Result<(), CatalogError> {
    *cursor = cursor.saturating_add(1);
    let mut keys = BTreeSet::new();
    loop {
        skip_space(bytes, cursor);
        if bytes.get(*cursor) == Some(&b'}') {
            *cursor = cursor.saturating_add(1);
            return Ok(());
        }
        let key_start = *cursor;
        scan_string(bytes, cursor)?;
        let key_bytes = bytes
            .get(key_start..*cursor)
            .ok_or_else(|| scan_error("invalid object key"))?;
        let key: String =
            serde_json::from_slice(key_bytes).map_err(|_error| scan_error("invalid object key"))?;
        if !keys.insert(key.clone()) {
            return Err(CatalogError::DuplicateKey { path, key });
        }
        skip_space(bytes, cursor);
        expect_byte(bytes, cursor, b':')?;
        scan_child(bytes, cursor, depth, path.as_str(), key)?;
        skip_space(bytes, cursor);
        if bytes.get(*cursor) == Some(&b',') {
            *cursor = cursor.saturating_add(1);
            continue;
        }
        expect_byte(bytes, cursor, b'}')?;
        return Ok(());
    }
}

fn scan_child(
    bytes: &[u8],
    cursor: &mut usize,
    depth: usize,
    path: &str,
    key: String,
) -> Result<(), CatalogError> {
    let child_path = if path.is_empty() {
        key
    } else {
        format!("{path}.{key}")
    };
    let child_depth = depth
        .checked_add(1)
        .ok_or_else(|| scan_error("depth overflow"))?;
    scan_value(bytes, cursor, child_depth, child_path)
}

fn scan_array(
    bytes: &[u8],
    cursor: &mut usize,
    depth: usize,
    path: &str,
) -> Result<(), CatalogError> {
    *cursor = cursor.saturating_add(1);
    let mut index = 0_usize;
    loop {
        skip_space(bytes, cursor);
        if bytes.get(*cursor) == Some(&b']') {
            *cursor = cursor.saturating_add(1);
            return Ok(());
        }
        let child_path = format!("{path}[{index}]");
        scan_value(
            bytes,
            cursor,
            depth
                .checked_add(1)
                .ok_or_else(|| scan_error("depth overflow"))?,
            child_path,
        )?;
        index = index
            .checked_add(1)
            .ok_or_else(|| scan_error("array index overflow"))?;
        skip_space(bytes, cursor);
        if bytes.get(*cursor) == Some(&b',') {
            *cursor = cursor.saturating_add(1);
            continue;
        }
        expect_byte(bytes, cursor, b']')?;
        return Ok(());
    }
}

fn scan_string(bytes: &[u8], cursor: &mut usize) -> Result<(), CatalogError> {
    expect_byte(bytes, cursor, b'"')?;
    let mut escaped = false;
    while let Some(byte) = bytes.get(*cursor).copied() {
        *cursor = cursor.saturating_add(1);
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == b'"' {
            return Ok(());
        }
    }
    Err(scan_error("unterminated string"))
}

fn scan_scalar(bytes: &[u8], cursor: &mut usize) {
    while let Some(byte) = bytes.get(*cursor).copied() {
        if matches!(byte, b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t') {
            return;
        }
        *cursor = cursor.saturating_add(1);
    }
}

fn skip_space(bytes: &[u8], cursor: &mut usize) {
    while matches!(bytes.get(*cursor), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        *cursor = cursor.saturating_add(1);
    }
}

fn expect_byte(bytes: &[u8], cursor: &mut usize, expected: u8) -> Result<(), CatalogError> {
    if bytes.get(*cursor) == Some(&expected) {
        *cursor = cursor.saturating_add(1);
        Ok(())
    } else {
        Err(scan_error("invalid JSON punctuation"))
    }
}

fn scan_error(message: &str) -> CatalogError {
    match message.len() {
        0 => CatalogError::Scan(String::new()),
        _ => CatalogError::Scan(message.to_owned()),
    }
}
