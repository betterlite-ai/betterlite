use std::panic::{AssertUnwindSafe, catch_unwind};

use lopdf::content::Content;
use lopdf::{Document, Encoding, LoadOptions, Object};

const MAX_DECOMPRESSED_PDF_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_EXTRACTED_PDF_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[must_use]
pub fn extract_text_from_pdf(file_content: &[u8]) -> Option<String> {
    catch_unwind(AssertUnwindSafe(|| {
        let options = LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED_PDF_BYTES);
        let document = Document::load_mem_with_options(file_content, options).ok()?;
        let mut output = String::new();
        for page_id in document.get_pages().values().copied() {
            let page_text = extract_page_text(&document, page_id)?;
            let separator = if output.is_empty() || page_text.is_empty() {
                ""
            } else {
                "\n\n"
            };
            let needed = output
                .len()
                .checked_add(separator.len())?
                .checked_add(page_text.len())?;
            if needed > MAX_EXTRACTED_PDF_TEXT_BYTES {
                return None;
            }
            output.push_str(separator);
            output.push_str(&page_text);
        }
        (!output.is_empty()).then_some(output)
    }))
    .unwrap_or_default()
}

fn extract_page_text(document: &Document, page_id: lopdf::ObjectId) -> Option<String> {
    let fonts = document.get_page_fonts(page_id).ok()?;
    let encodings: Vec<(Vec<u8>, Encoding<'_>)> = fonts
        .into_iter()
        .filter_map(|(name, font)| {
            font.get_font_encoding_with_limit(document, MAX_DECOMPRESSED_PDF_BYTES)
                .ok()
                .map(|encoding| (name, encoding))
        })
        .collect();
    let data = document
        .get_page_content_with_limit(page_id, MAX_DECOMPRESSED_PDF_BYTES)
        .ok()?;
    let content = Content::decode(&data).ok()?;
    let mut text = String::new();
    let mut font: Option<&Encoding<'_>> = None;
    let mut font_size = 0.0_f64;
    let mut scale_y = 1.0_f64;
    let mut leading = 0.0_f64;
    let mut y = 0.0_f64;
    let mut previous_y: Option<f64> = None;
    let mut line_start_y = 0.0_f64;
    let mut line_end_y = 0.0_f64;
    for operation in content.operations {
        match operation.operator.as_str() {
            "Tf" => {
                let name = operation.operands.first()?.as_name().ok()?;
                font = encodings
                    .iter()
                    .find(|(candidate, _)| candidate.as_slice() == name)
                    .map(|(_, encoding)| encoding);
                font_size = operation.operands.get(1).and_then(number).unwrap_or(0.0);
            }
            "TL" => leading = operation.operands.first().and_then(number)?,
            "Td" | "TD" => {
                let tx = operation.operands.first().and_then(number)?;
                let ty = operation.operands.get(1).and_then(number)?;
                if operation.operator == "TD" {
                    leading = -ty;
                }
                move_text(
                    &mut text,
                    &mut previous_y,
                    &mut line_start_y,
                    &mut line_end_y,
                    y,
                    y + ty,
                    font_size * scale_y,
                );
                let _ = tx;
                y += ty;
            }
            "Tm" => {
                let matrix = operation
                    .operands
                    .iter()
                    .filter_map(number)
                    .collect::<Vec<_>>();
                let new_x = *matrix.get(4)?;
                let new_y = *matrix.get(5)?;
                scale_y = matrix.get(3)?.abs();
                move_text(
                    &mut text,
                    &mut previous_y,
                    &mut line_start_y,
                    &mut line_end_y,
                    y,
                    new_y,
                    font_size * scale_y,
                );
                let _ = new_x;
                y = new_y;
            }
            "T*" => {
                let next_y = y - leading;
                move_text(
                    &mut text,
                    &mut previous_y,
                    &mut line_start_y,
                    &mut line_end_y,
                    y,
                    next_y,
                    font_size * scale_y,
                );
                y = next_y;
            }
            "Tj" => append_text(&mut text, font, operation.operands.first()?)?,
            "TJ" => {
                let values = operation.operands.first()?.as_array().ok()?;
                for value in values {
                    match value {
                        Object::String(bytes, _) => append_bytes(&mut text, font?, &bytes)?,
                        Object::Integer(offset) if *offset < -100 => text.push(' '),
                        _ => {}
                    }
                }
            }
            "'" => {
                let next_y = y - leading;
                move_text(
                    &mut text,
                    &mut previous_y,
                    &mut line_start_y,
                    &mut line_end_y,
                    y,
                    next_y,
                    font_size * scale_y,
                );
                y = next_y;
                append_text(&mut text, font, operation.operands.first()?)?;
            }
            "\"" => {
                let next_y = y - leading;
                move_text(
                    &mut text,
                    &mut previous_y,
                    &mut line_start_y,
                    &mut line_end_y,
                    y,
                    next_y,
                    font_size * scale_y,
                );
                y = next_y;
                append_text(&mut text, font, operation.operands.get(2)?)?;
            }
            "ET" | "BT" => {}
            _ => {}
        }
        if text.len() > MAX_EXTRACTED_PDF_TEXT_BYTES {
            return None;
        }
    }
    Some(text)
}

fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => value.to_string().parse().ok(),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    }
}

fn append_text(text: &mut String, encoding: Option<&Encoding<'_>>, object: &Object) -> Option<()> {
    match object {
        Object::String(bytes, _) => append_bytes(text, encoding?, bytes),
        _ => None,
    }
}

fn append_bytes(text: &mut String, encoding: &Encoding<'_>, bytes: &[u8]) -> Option<()> {
    text.push_str(&Document::decode_text(encoding, bytes).ok()?);
    Some(())
}

fn move_text(
    text: &mut String,
    previous_y: &mut Option<f64>,
    line_start_y: &mut f64,
    line_end_y: &mut f64,
    old_y: f64,
    new_y: f64,
    font_size: f64,
) {
    let Some(previous) = *previous_y else {
        *previous_y = Some(new_y);
        *line_start_y = new_y;
        *line_end_y = new_y;
        return;
    };
    let low = line_start_y.min(*line_end_y);
    let high = line_start_y.max(*line_end_y);
    let distance = if new_y > high {
        new_y - high
    } else if new_y < low {
        low - new_y
    } else {
        0.0
    };
    if distance > 0.8 * font_size.abs() {
        text.push('\n');
        *line_start_y = new_y;
        *line_end_y = new_y;
    } else {
        *line_start_y = line_start_y.min(new_y);
        *line_end_y = line_end_y.max(new_y);
    }
    *previous_y = Some(previous + (new_y - old_y));
}
