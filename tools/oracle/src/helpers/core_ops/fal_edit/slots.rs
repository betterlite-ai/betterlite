use std::io::Cursor;
use std::path::PathBuf;
use std::slice::{Iter, IterMut};

use betterlite_core::api::fal::edit::image_input::EditImage;

use crate::helpers::HelperError;

use super::input::PrepareImage;

#[derive(Default)]
pub(super) struct Sources {
    contents: Vec<Vec<u8>>,
    texts: Vec<String>,
    paths: Vec<PathBuf>,
    tuples: Vec<Vec<u8>>,
}

impl Sources {
    pub(super) fn inners<'value>(&'value self) -> Vec<EditImage<'value>> {
        let mut inners = Vec::with_capacity(self.tuples.len());
        for content in &self.tuples {
            inners.push(EditImage::Bytes(content));
        }
        inners
    }
}

type SlotImages<'value> = Vec<Option<EditImage<'value>>>;

pub(super) enum Slot {
    Absent,
    Bytes,
    Text,
    Path,
    Reader,
    Tuple,
}

struct Slots<'value> {
    contents: Iter<'value, Vec<u8>>,
    texts: Iter<'value, String>,
    paths: Iter<'value, PathBuf>,
    readers: IterMut<'value, Cursor<Vec<u8>>>,
    inners: IterMut<'value, EditImage<'value>>,
}

impl<'value> Slots<'value> {
    fn image(&mut self, slot: &Slot) -> Result<Option<EditImage<'value>>, Box<HelperError>> {
        match slot {
            Slot::Absent => Ok(None),
            Slot::Bytes => Ok(Some(EditImage::Bytes(next_source(
                &mut self.contents,
                "images",
            )?))),
            Slot::Text => Ok(Some(EditImage::Text(next_source(
                &mut self.texts,
                "texts",
            )?))),
            Slot::Path => Ok(Some(EditImage::Path(next_source(
                &mut self.paths,
                "paths",
            )?))),
            Slot::Reader => Ok(Some(EditImage::Reader(next_source(
                &mut self.readers,
                "readers",
            )?))),
            Slot::Tuple => Ok(Some(EditImage::Tuple(next_source(
                &mut self.inners,
                "tuples",
            )?))),
        }
    }
}

fn next_source<Source: Iterator>(
    sources: &mut Source,
    kind: &str,
) -> Result<Source::Item, Box<HelperError>> {
    sources
        .next()
        .ok_or_else(|| helper_error(format!("the prepare case lists more {kind} than it holds")))
}

pub(super) fn collect_slots(
    entries: &[Option<PrepareImage>],
    sources: &mut Sources,
    readers: &mut Vec<Cursor<Vec<u8>>>,
) -> Result<Vec<Slot>, Box<HelperError>> {
    let mut slots = Vec::with_capacity(entries.len());
    for entry in entries {
        slots.push(collect_slot(entry.as_ref(), sources, readers)?);
    }
    Ok(slots)
}

fn collect_slot(
    entry: Option<&PrepareImage>,
    sources: &mut Sources,
    readers: &mut Vec<Cursor<Vec<u8>>>,
) -> Result<Slot, Box<HelperError>> {
    Ok(match entry {
        None => Slot::Absent,
        Some(PrepareImage::Bytes { content }) => {
            sources.contents.push(content.clone());
            Slot::Bytes
        }
        Some(PrepareImage::Text { text }) => {
            sources.texts.push(text.clone());
            Slot::Text
        }
        Some(PrepareImage::Path { name, content }) => {
            sources.paths.push(fixture_path(name, content)?);
            Slot::Path
        }
        Some(PrepareImage::Reader { content, position }) => {
            let mut reader = Cursor::new(content.clone());
            reader.set_position(*position);
            readers.push(reader);
            Slot::Reader
        }
        Some(PrepareImage::Tuple { content }) => {
            sources.tuples.push(content.clone());
            Slot::Tuple
        }
    })
}

pub(super) fn request_arrays<'value>(
    sources: &'value Sources,
    readers: &'value mut [Cursor<Vec<u8>>],
    inners: &'value mut [EditImage<'value>],
    image_slots: &[Slot],
    mask_slots: &[Slot],
) -> Result<(SlotImages<'value>, SlotImages<'value>), Box<HelperError>> {
    let mut slots = Slots {
        contents: sources.contents.iter(),
        texts: sources.texts.iter(),
        paths: sources.paths.iter(),
        readers: readers.iter_mut(),
        inners: inners.iter_mut(),
    };
    let images = build_array(&mut slots, image_slots)?;
    let mask = build_array(&mut slots, mask_slots)?;
    Ok((images, mask))
}

fn build_array<'value>(
    slots: &mut Slots<'value>,
    cases: &[Slot],
) -> Result<SlotImages<'value>, Box<HelperError>> {
    let mut images = Vec::with_capacity(cases.len());
    for case in cases {
        images.push(slots.image(case)?);
    }
    Ok(images)
}

pub(super) fn reader_positions(
    image_slots: &[Slot],
    mask_slots: &[Slot],
    readers: &[Cursor<Vec<u8>>],
) -> (Vec<Option<u64>>, Vec<Option<u64>>) {
    let mut positions = readers.iter();
    let image_positions = slot_positions(image_slots, &mut positions);
    let mask_positions = slot_positions(mask_slots, &mut positions);
    (image_positions, mask_positions)
}

fn slot_positions(slots: &[Slot], readers: &mut Iter<'_, Cursor<Vec<u8>>>) -> Vec<Option<u64>> {
    let mut positions = Vec::with_capacity(slots.len());
    for slot in slots {
        positions.push(match slot {
            Slot::Reader => readers.next().map(Cursor::position),
            Slot::Absent | Slot::Bytes | Slot::Text | Slot::Path | Slot::Tuple => None,
        });
    }
    positions
}

pub(super) fn fixture_path(name: &str, content: &[u8]) -> Result<PathBuf, Box<HelperError>> {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, content).map_err(|error| helper_error(error.to_string()))?;
    Ok(path)
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
