mod position;
mod tests;

use std::panic::{AssertUnwindSafe, catch_unwind};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Encoding, LoadOptions, Object};

use position::{Matrix, Position, number};

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
            append_page(&mut output, &page_text)?;
        }
        (!output.is_empty()).then_some(output)
    }))
    .unwrap_or_default()
}

fn append_page(output: &mut String, page_text: &str) -> Option<()> {
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
    output.push_str(page_text);
    Some(())
}

fn extract_page_text(document: &Document, page_id: lopdf::ObjectId) -> Option<String> {
    let fonts = page_encodings(document, page_id)?;
    let data = document
        .get_page_content_with_limit(page_id, MAX_DECOMPRESSED_PDF_BYTES)
        .ok()?;
    let content = Content::decode(&data).ok()?;
    let mut extractor = PageText {
        text: String::new(),
        encodings: fonts,
        active_font: None,
        font_size: 0.0_f64,
        position: Position::default(),
        saved: Vec::new(),
    };
    for operation in &content.operations {
        extractor.apply(operation)?;
        if extractor.text.len() > MAX_EXTRACTED_PDF_TEXT_BYTES {
            return None;
        }
    }
    Some(extractor.text)
}

fn page_encodings(
    document: &Document,
    page_id: lopdf::ObjectId,
) -> Option<Vec<(Vec<u8>, Encoding<'_>)>> {
    let fonts = document.get_page_fonts(page_id).ok()?;
    Some(
        fonts
            .into_iter()
            .filter_map(|(name, font)| {
                font.get_font_encoding_with_limit(document, MAX_DECOMPRESSED_PDF_BYTES)
                    .ok()
                    .map(|encoding| (name, encoding))
            })
            .collect(),
    )
}

struct PageText<'doc> {
    text: String,
    encodings: Vec<(Vec<u8>, Encoding<'doc>)>,
    active_font: Option<usize>,
    font_size: f64,
    position: Position,
    saved: Vec<(Matrix, Option<usize>, f64, f64)>,
}

impl PageText<'_> {
    fn apply(&mut self, operation: &Operation) -> Option<()> {
        match operation.operator.as_str() {
            "Tf" => self.select_font(&operation.operands)?,
            "Tj" => self.show_text(operation.operands.first())?,
            "TJ" => self.show_text_array(operation.operands.first())?,
            "'" => self.show_next_line_text(operation.operands.first())?,
            "\"" if operation.operands.len() >= 3 => {
                self.show_next_line_text(operation.operands.get(2))?;
            }
            "q" => self.saved.push((
                self.position.graphics,
                self.active_font,
                self.font_size,
                self.position.leading,
            )),
            "Q" => self.restore_graphics(),
            _ => self
                .position
                .apply(operation, &mut self.text, self.font_size),
        }
        Some(())
    }

    fn restore_graphics(&mut self) {
        if let Some((matrix, font, size, leading)) = self.saved.pop() {
            self.position.graphics = matrix;
            self.active_font = font;
            self.font_size = size;
            self.position.leading = leading;
        }
    }

    fn select_font(&mut self, operands: &[Object]) -> Option<()> {
        let name = operands.first()?;
        self.active_font = self.encodings.iter().position(|(candidate, _)| {
            name.as_name()
                .is_ok_and(|value| candidate.as_slice() == value)
        });
        self.font_size = operands.get(1).and_then(number).unwrap_or(self.font_size);
        Some(())
    }

    fn show_next_line_text(&mut self, object: Option<&Object>) -> Option<()> {
        self.position.next_line(&mut self.text, self.font_size);
        self.show_text(object)
    }

    fn show_text(&mut self, object: Option<&Object>) -> Option<()> {
        let Some(bytes) = object.and_then(|value| value.as_str().ok()) else {
            return Some(());
        };
        let Some((_, encoding)) = self.active_font.and_then(|index| self.encodings.get(index))
        else {
            return Some(());
        };
        let decoded = Document::decode_text(encoding, bytes).ok()?;
        self.text.push_str(&decoded);
        self.position.height = self.font_size;
        self.position.check(&mut self.text, self.font_size);
        Some(())
    }

    fn show_text_array(&mut self, object: Option<&Object>) -> Option<()> {
        let Some(values) = object.and_then(|value| value.as_array().ok()) else {
            return Some(());
        };
        for value in values {
            match value {
                Object::String(_, _) => self.show_text(Some(value))?,
                Object::Integer(offset) if *offset < -100 => self.text.push(' '),
                Object::Integer(_)
                | Object::Real(_)
                | Object::Boolean(_)
                | Object::Name(_)
                | Object::Array(_)
                | Object::Dictionary(_)
                | Object::Stream(_)
                | Object::Null
                | Object::Reference(_) => {}
            }
        }
        self.text.push(' ');
        Some(())
    }
}
