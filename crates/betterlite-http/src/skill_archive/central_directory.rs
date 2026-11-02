use std::io::{Cursor, Read as _};
use std::ops::Range;

use zip::ZipArchive;

use super::{
    CENTRAL_HEADER_FIXED_LEN, CENTRAL_HEADER_FLAGS_RANGE, CENTRAL_HEADER_NAME_LEN_RANGE,
    CENTRAL_HEADER_SIGNATURE, LOCAL_HEADER_FIXED_LEN, LOCAL_HEADER_NAME_LEN_RANGE,
    LOCAL_HEADER_SIGNATURE, decode_header_name, header_name_bytes,
};

const CENTRAL_COMPRESSED_LEN_RANGE: Range<usize> = 20..24;
const CENTRAL_UNCOMPRESSED_LEN_RANGE: Range<usize> = 24..28;
const CENTRAL_HEADER_EXTRA_LEN_RANGE: Range<usize> = 30..32;
const CENTRAL_COMMENT_LEN_RANGE: Range<usize> = 32..34;
const CENTRAL_LOCAL_OFFSET_RANGE: Range<usize> = 42..46;
const LOCAL_HEADER_EXTRA_LEN_RANGE: Range<usize> = 28..30;
const ZIP64_EXTRA_FIELD_TAG: u16 = 0x0001;
const ZIP64_SENTINEL: u32 = 0xFFFF_FFFF;

pub(super) struct CentralEntry {
    pub(super) name: String,
    pub(super) local_header_start: u64,
    pub(super) declared_uncompressed_size: u64,
}

// Walks the central directory by byte offset, bypassing the zip crate's name-keyed index so a 0x7075 collision can't hide a member.
pub(super) fn header_starts(stored_content: &[u8], dir_start: u64) -> Vec<u64> {
    // uploaded.len() is itself keyed by decoded name, so it would undercount the same collision; bound by byte length instead.
    let max_entries = stored_content
        .len()
        .checked_div(CENTRAL_HEADER_FIXED_LEN)
        .and_then(|count| count.checked_add(1))
        .unwrap_or(stored_content.len());
    let mut starts = Vec::new();
    let mut offset = dir_start;
    for _ in 0..max_entries {
        let Some(next) = next_header_start(stored_content, offset) else {
            break;
        };
        starts.push(offset);
        offset = next;
    }
    starts
}

fn next_header_start(stored_content: &[u8], header_start: u64) -> Option<u64> {
    let (fixed, _name) = header_name_bytes(
        stored_content,
        header_start,
        CENTRAL_HEADER_FIXED_LEN,
        CENTRAL_HEADER_SIGNATURE,
        CENTRAL_HEADER_NAME_LEN_RANGE,
    )?;
    let name_len = u16::from_le_bytes(fixed.get(CENTRAL_HEADER_NAME_LEN_RANGE)?.try_into().ok()?);
    let extra_len = u16::from_le_bytes(fixed.get(CENTRAL_HEADER_EXTRA_LEN_RANGE)?.try_into().ok()?);
    let comment_len = u16::from_le_bytes(fixed.get(CENTRAL_COMMENT_LEN_RANGE)?.try_into().ok()?);
    let total = usize::from(name_len)
        .checked_add(usize::from(extra_len))?
        .checked_add(usize::from(comment_len))?;
    header_start.checked_add(u64::try_from(CENTRAL_HEADER_FIXED_LEN.checked_add(total)?).ok()?)
}

pub(super) fn read_entry(
    stored_content: &[u8],
    central_header_start: u64,
    archive_offset: u64,
) -> Option<CentralEntry> {
    let (fixed, name_raw) = header_name_bytes(
        stored_content,
        central_header_start,
        CENTRAL_HEADER_FIXED_LEN,
        CENTRAL_HEADER_SIGNATURE,
        CENTRAL_HEADER_NAME_LEN_RANGE,
    )?;
    let flags = u16::from_le_bytes(fixed.get(CENTRAL_HEADER_FLAGS_RANGE)?.try_into().ok()?);
    let name = decode_header_name(name_raw, flags & super::LANGUAGE_ENCODING_FLAG != 0)?;
    let central_record = central_record_bytes(stored_content, central_header_start)?;
    let (declared_uncompressed_size, _compressed_size, resolved_offset) =
        central_zip64_fields(central_record)?;
    Some(CentralEntry {
        name,
        local_header_start: resolved_offset.checked_add(archive_offset)?,
        declared_uncompressed_size,
    })
}

// Reads a central record's size and offset fields, resolving the zip64 extra field when a field holds the 0xFFFFFFFF sentinel.
fn central_zip64_fields(central_record: &[u8]) -> Option<(u64, u64, u64)> {
    let uncompressed_size = u32::from_le_bytes(
        central_record
            .get(CENTRAL_UNCOMPRESSED_LEN_RANGE)?
            .try_into()
            .ok()?,
    );
    let compressed_size = u32::from_le_bytes(
        central_record
            .get(CENTRAL_COMPRESSED_LEN_RANGE)?
            .try_into()
            .ok()?,
    );
    let local_header_offset = u32::from_le_bytes(
        central_record
            .get(CENTRAL_LOCAL_OFFSET_RANGE)?
            .try_into()
            .ok()?,
    );
    let name_len = usize::from(u16::from_le_bytes(
        central_record
            .get(CENTRAL_HEADER_NAME_LEN_RANGE)?
            .try_into()
            .ok()?,
    ));
    let extra_len = usize::from(u16::from_le_bytes(
        central_record
            .get(CENTRAL_HEADER_EXTRA_LEN_RANGE)?
            .try_into()
            .ok()?,
    ));
    let extra_start = CENTRAL_HEADER_FIXED_LEN.checked_add(name_len)?;
    let extra = central_record.get(extra_start..extra_start.checked_add(extra_len)?)?;
    resolve_zip64_fields(
        uncompressed_size,
        compressed_size,
        local_header_offset,
        extra,
    )
}

// Follows zipfile._decodeExtra's order: a central field that reads as the zip64 sentinel is replaced by the next unread u64 in the 0x0001 extra field (uncompressed size, then compressed size, then local header offset).
fn resolve_zip64_fields(
    raw_uncompressed_size: u32,
    raw_compressed_size: u32,
    raw_local_header_offset: u32,
    extra: &[u8],
) -> Option<(u64, u64, u64)> {
    let raw_fields = [
        raw_uncompressed_size,
        raw_compressed_size,
        raw_local_header_offset,
    ];
    if !raw_fields.contains(&ZIP64_SENTINEL) {
        return Some((
            u64::from(raw_uncompressed_size),
            u64::from(raw_compressed_size),
            u64::from(raw_local_header_offset),
        ));
    }
    let zip64_field = find_extra_field(extra, ZIP64_EXTRA_FIELD_TAG)?;
    let mut resolved = [0_u64; 3];
    let mut cursor = 0_usize;
    for (slot, raw_field) in resolved.iter_mut().zip(raw_fields) {
        *slot = if raw_field == ZIP64_SENTINEL {
            let value = u64::from_le_bytes(
                zip64_field
                    .get(cursor..cursor.checked_add(8)?)?
                    .try_into()
                    .ok()?,
            );
            cursor = cursor.checked_add(8)?;
            value
        } else {
            u64::from(raw_field)
        };
    }
    Some(<(u64, u64, u64)>::from(resolved))
}

fn find_extra_field(extra: &[u8], tag: u16) -> Option<&[u8]> {
    let mut offset = 0_usize;
    while let Some(header) = extra.get(offset..offset.checked_add(4)?) {
        let field_tag = u16::from_le_bytes(header.get(0..2)?.try_into().ok()?);
        let field_len = usize::from(u16::from_le_bytes(header.get(2..4)?.try_into().ok()?));
        let data_start = offset.checked_add(4)?;
        let data_end = data_start.checked_add(field_len)?;
        let data = extra.get(data_start..data_end)?;
        if field_tag == tag {
            return Some(data);
        }
        offset = data_end;
    }
    None
}

// Copies one member's local header, data and own central record into a fresh single-entry zip, so decompressing it never touches the crate's shared index.
pub(super) fn decompress(
    stored_content: &[u8],
    central_header_start: u64,
    archive_offset: u64,
) -> Option<Vec<u8>> {
    let synthetic = single_entry_archive(stored_content, central_header_start, archive_offset)?;
    let mut archive = ZipArchive::new(Cursor::new(synthetic.as_slice())).ok()?;
    let mut data = Vec::new();
    archive.by_index(0).ok()?.read_to_end(&mut data).ok()?;
    Some(data)
}

fn single_entry_archive(
    stored_content: &[u8],
    central_header_start: u64,
    archive_offset: u64,
) -> Option<Vec<u8>> {
    let central_record = central_record_bytes(stored_content, central_header_start)?;
    let local_record = local_record_bytes(stored_content, central_record, archive_offset)?;
    assemble_synthetic_archive(local_record, central_record)
}

fn central_record_bytes(stored_content: &[u8], central_header_start: u64) -> Option<&[u8]> {
    let start = usize::try_from(central_header_start).ok()?;
    let fixed = stored_content.get(start..start.checked_add(CENTRAL_HEADER_FIXED_LEN)?)?;
    let name_len = usize::from(u16::from_le_bytes(
        fixed.get(CENTRAL_HEADER_NAME_LEN_RANGE)?.try_into().ok()?,
    ));
    let extra_len = usize::from(u16::from_le_bytes(
        fixed.get(CENTRAL_HEADER_EXTRA_LEN_RANGE)?.try_into().ok()?,
    ));
    let comment_len = usize::from(u16::from_le_bytes(
        fixed.get(CENTRAL_COMMENT_LEN_RANGE)?.try_into().ok()?,
    ));
    let total = CENTRAL_HEADER_FIXED_LEN
        .checked_add(name_len)?
        .checked_add(extra_len)?
        .checked_add(comment_len)?;
    stored_content.get(start..start.checked_add(total)?)
}

fn local_record_bytes<'content>(
    stored_content: &'content [u8],
    central_record: &[u8],
    archive_offset: u64,
) -> Option<&'content [u8]> {
    let (_uncompressed_size, resolved_compressed_size, resolved_offset) =
        central_zip64_fields(central_record)?;
    let local_header_start = resolved_offset.checked_add(archive_offset)?;
    let (local_fixed, _local_name) = header_name_bytes(
        stored_content,
        local_header_start,
        LOCAL_HEADER_FIXED_LEN,
        LOCAL_HEADER_SIGNATURE,
        LOCAL_HEADER_NAME_LEN_RANGE,
    )?;
    let local_name_len = usize::from(u16::from_le_bytes(
        local_fixed
            .get(LOCAL_HEADER_NAME_LEN_RANGE)?
            .try_into()
            .ok()?,
    ));
    let local_extra_len = usize::from(u16::from_le_bytes(
        local_fixed
            .get(LOCAL_HEADER_EXTRA_LEN_RANGE)?
            .try_into()
            .ok()?,
    ));
    let local_start = usize::try_from(local_header_start).ok()?;
    let local_total = LOCAL_HEADER_FIXED_LEN
        .checked_add(local_name_len)?
        .checked_add(local_extra_len)?
        .checked_add(usize::try_from(resolved_compressed_size).ok()?)?;
    stored_content.get(local_start..local_start.checked_add(local_total)?)
}

fn assemble_synthetic_archive(local_record: &[u8], central_record: &[u8]) -> Option<Vec<u8>> {
    let capacity = local_record
        .len()
        .checked_add(central_record.len())?
        .checked_add(22)?;
    let mut synthetic = Vec::with_capacity(capacity);
    synthetic.extend_from_slice(local_record);
    let cd_offset = u32::try_from(synthetic.len()).ok()?;
    synthetic.extend_from_slice(central_record);
    patch_local_header_offset(&mut synthetic, cd_offset)?;
    let cd_size = u32::try_from(central_record.len()).ok()?;
    append_eocd(&mut synthetic, cd_size, cd_offset);
    Some(synthetic)
}

fn patch_local_header_offset(synthetic: &mut [u8], cd_offset: u32) -> Option<()> {
    let field_start = usize::try_from(cd_offset)
        .ok()?
        .checked_add(CENTRAL_LOCAL_OFFSET_RANGE.start)?;
    let field_end = field_start.checked_add(4)?;
    synthetic
        .get_mut(field_start..field_end)?
        .copy_from_slice(&0_u32.to_le_bytes());
    Some(())
}

fn append_eocd(synthetic: &mut Vec<u8>, cd_size: u32, cd_offset: u32) {
    synthetic.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    synthetic.extend_from_slice(&0_u16.to_le_bytes());
    synthetic.extend_from_slice(&0_u16.to_le_bytes());
    synthetic.extend_from_slice(&1_u16.to_le_bytes());
    synthetic.extend_from_slice(&1_u16.to_le_bytes());
    synthetic.extend_from_slice(&cd_size.to_le_bytes());
    synthetic.extend_from_slice(&cd_offset.to_le_bytes());
    synthetic.extend_from_slice(&0_u16.to_le_bytes());
}
