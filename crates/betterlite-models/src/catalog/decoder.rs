use std::collections::BTreeSet;

use super::{CatalogError, MAX_CATALOG_DEPTH};

#[derive(Clone, Copy)]
enum Container {
    Object,
    Array,
}

pub(super) fn check_duplicates(bytes: &[u8]) -> Result<(), CatalogError> {
    let mut cursor = 0;
    scan_value(bytes, &mut cursor, 0, "")?;
    Ok(())
}

fn scan_value(
    bytes: &[u8],
    cursor: &mut usize,
    depth: usize,
    path: &str,
) -> Result<(), CatalogError> {
    if depth >= MAX_CATALOG_DEPTH {
        return Err(scan_error("model catalog nesting exceeds limit"));
    }
    skip_space(bytes, cursor);
    let byte = bytes
        .get(*cursor)
        .copied()
        .ok_or_else(|| scan_error("unexpected end"))?;
    scan_value_kind(bytes, cursor, depth, path, byte)
}

fn scan_value_kind(
    bytes: &[u8],
    cursor: &mut usize,
    depth: usize,
    path: &str,
    byte: u8,
) -> Result<(), CatalogError> {
    if byte == b'{' {
        return scan_container(bytes, cursor, depth, path, Container::Object);
    }
    if byte == b'[' {
        return scan_container(bytes, cursor, depth, path, Container::Array);
    }
    if byte == b'"' {
        scan_string(bytes, cursor)?;
    } else {
        scan_scalar(bytes, cursor);
    }
    Ok(())
}

fn scan_container(
    bytes: &[u8],
    cursor: &mut usize,
    depth: usize,
    path: &str,
    kind: Container,
) -> Result<(), CatalogError> {
    let close = match kind {
        Container::Object => b'}',
        Container::Array => b']',
    };
    *cursor = cursor.saturating_add(1);
    let mut keys = BTreeSet::new();
    let mut index = 0_usize;
    loop {
        skip_space(bytes, cursor);
        if bytes.get(*cursor) == Some(&close) {
            *cursor = cursor.saturating_add(1);
            return Ok(());
        }
        let child_path = match kind {
            Container::Object => scan_object_key(bytes, cursor, path, &mut keys)?,
            Container::Array => {
                let value = format!("{path}[{index}]");
                index = index
                    .checked_add(1)
                    .ok_or_else(|| scan_error("array index overflow"))?;
                value
            }
        };
        let child_depth = depth
            .checked_add(1)
            .ok_or_else(|| scan_error("depth overflow"))?;
        scan_value(bytes, cursor, child_depth, &child_path)?;
        skip_space(bytes, cursor);
        if bytes.get(*cursor) == Some(&b',') {
            *cursor = cursor.saturating_add(1);
            continue;
        }
        expect_byte(bytes, cursor, close)?;
        return Ok(());
    }
}

fn scan_object_key(
    bytes: &[u8],
    cursor: &mut usize,
    path: &str,
    keys: &mut BTreeSet<String>,
) -> Result<String, CatalogError> {
    let key_start = *cursor;
    scan_string(bytes, cursor)?;
    let key_bytes = bytes
        .get(key_start..*cursor)
        .ok_or_else(|| scan_error("invalid object key"))?;
    let key: String =
        serde_json::from_slice(key_bytes).map_err(|_error| scan_error("invalid object key"))?;
    if !keys.insert(key.clone()) {
        return Err(CatalogError::DuplicateKey {
            path: path.to_owned(),
            key,
        });
    }
    skip_space(bytes, cursor);
    expect_byte(bytes, cursor, b':')?;
    Ok(if path.is_empty() {
        key
    } else {
        format!("{path}.{key}")
    })
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
