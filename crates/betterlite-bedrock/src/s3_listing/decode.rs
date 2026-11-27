use std::mem;

use betterlite_http::proxy::limits::TransportLimits;
use chrono::{DateTime, Datelike as _};
use quick_xml::Error as XmlError;
use quick_xml::NsReader;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::name::{NamespaceResolver, ResolveResult};

use crate::image_models::sizing::parse_python_int;

use super::S3ListingError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    pub body_bytes: usize,
    pub depth: usize,
}

impl DecodeLimits {
    pub const DEFAULT: Self = Self {
        body_bytes: TransportLimits::DEFAULT.body_bytes,
        depth: 64,
    };
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S3Listing {
    pub name: Option<String>,
    pub contents: Vec<S3ListedEntry>,
    pub next_continuation_token: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S3ListedEntry {
    pub key: Option<String>,
    pub size: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    BucketName,
    Token,
    Key,
    Size,
    Modified,
}

// ElementTree keeps an element's text only up to its first child, and only the root's direct children count
#[derive(Default)]
struct Parser {
    depth: usize,
    root_seen: bool,
    in_contents: bool,
    slot: Option<Slot>,
    slot_depth: usize,
    has_child: bool,
    text: Option<String>,
    listing: S3Listing,
    entry: S3ListedEntry,
}

impl Parser {
    fn open(&mut self, local: &[u8], limits: &DecodeLimits) -> Result<(), S3ListingError> {
        if self.depth >= limits.depth {
            return Err(S3ListingError::DepthExceeded {
                limit: limits.depth,
            });
        }
        if self.depth == 0 && self.root_seen {
            return Err(S3ListingError::Trailing);
        }
        self.depth = self.depth.saturating_add(1);
        if self.slot.is_some() {
            self.has_child = true;
            return Ok(());
        }
        let slot = match (self.depth, local) {
            (2, b"Name") => Some(Slot::BucketName),
            (2, b"NextContinuationToken") => Some(Slot::Token),
            (2, b"Contents") => {
                self.in_contents = true;
                self.entry = S3ListedEntry::default();
                None
            }
            (3, b"Key") if self.in_contents => Some(Slot::Key),
            (3, b"Size") if self.in_contents => Some(Slot::Size),
            (3, b"LastModified") if self.in_contents => Some(Slot::Modified),
            _ => None,
        };
        if slot.is_some() {
            self.slot = slot;
            self.slot_depth = self.depth;
            self.has_child = false;
            self.text = None;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), S3ListingError> {
        if self.depth == 0 {
            return Err(S3ListingError::Trailing);
        }
        if self.slot_depth == self.depth
            && let Some(slot) = self.slot.take()
        {
            let text = self.text.take();
            self.has_child = false;
            self.store(slot, text);
        }
        let closing = self.depth;
        self.depth = closing.saturating_sub(1);
        if closing == 2 && self.in_contents {
            self.in_contents = false;
            self.listing.contents.push(mem::take(&mut self.entry));
        }
        if self.depth == 0 {
            self.root_seen = true;
        }
        Ok(())
    }

    fn push_text(&mut self, content: &str) -> Result<(), S3ListingError> {
        if self.depth == 0 {
            if content.chars().all(char::is_whitespace) {
                return Ok(());
            }
            return Err(S3ListingError::Trailing);
        }
        if self.slot.is_some() && !self.has_child {
            self.text.get_or_insert_default().push_str(content);
        }
        Ok(())
    }

    fn reference(&mut self, node: &BytesRef<'_>) -> Result<(), S3ListingError> {
        let resolved = match node.resolve_char_ref()? {
            Some(character) => {
                let mut encoded = [0_u8; 4];
                character.encode_utf8(&mut encoded).to_owned()
            }
            None => {
                let name = node.decode()?;
                let replacement = match name.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "apos" => "'",
                    "quot" => "\"",
                    _ => {
                        return Err(S3ListingError::Entity {
                            name: name.into_owned(),
                        });
                    }
                };
                replacement.to_owned()
            }
        };
        self.push_text(&resolved)
    }

    fn store(&mut self, slot: Slot, text: Option<String>) {
        let target = match slot {
            Slot::BucketName => &mut self.listing.name,
            Slot::Token => &mut self.listing.next_continuation_token,
            Slot::Key => &mut self.entry.key,
            Slot::Size => &mut self.entry.size,
            Slot::Modified => &mut self.entry.last_modified,
        };
        if target.is_none() {
            *target = text;
        }
    }

    fn finish(self) -> Result<S3Listing, S3ListingError> {
        if self.depth != 0 {
            return Err(S3ListingError::Unclosed);
        }
        if !self.root_seen {
            return Err(S3ListingError::NoRoot);
        }
        Ok(self.listing)
    }
}

// ElementTree rejects an element or attribute name whose prefix was never declared
fn check_attributes(
    resolver: &NamespaceResolver,
    start: &BytesStart<'_>,
) -> Result<(), S3ListingError> {
    for attribute in start.attributes() {
        let resolved = attribute.map_err(XmlError::from)?;
        let Some(prefix) = resolved.key.prefix() else {
            continue;
        };
        if let ResolveResult::Unknown(unknown) = resolver.resolve_prefix(Some(prefix), false) {
            return Err(S3ListingError::UnboundPrefix {
                prefix: prefix_text(&unknown),
            });
        }
    }
    Ok(())
}

fn prefix_text(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

pub fn decode_s3_listing(bytes: &[u8], limits: &DecodeLimits) -> Result<S3Listing, S3ListingError> {
    if bytes.len() > limits.body_bytes {
        return Err(S3ListingError::BodyTooLarge {
            limit: limits.body_bytes,
        });
    }
    let mut reader = NsReader::from_reader(bytes);
    let mut buffer = Vec::new();
    let mut parser = Parser::default();
    loop {
        buffer.clear();
        let (resolved, event) = reader.read_resolved_event_into(&mut buffer)?;
        if let ResolveResult::Unknown(prefix) = &resolved {
            return Err(S3ListingError::UnboundPrefix {
                prefix: prefix_text(prefix),
            });
        }
        match event {
            Event::Start(start) => {
                check_attributes(reader.resolver(), &start)?;
                parser.open(start.local_name().as_ref(), limits)?;
            }
            Event::Empty(start) => {
                check_attributes(reader.resolver(), &start)?;
                parser.open(start.local_name().as_ref(), limits)?;
                parser.close()?;
            }
            Event::End(_end) => parser.close()?,
            Event::Text(text) => parser.push_text(&text.xml10_content()?)?,
            Event::CData(text) => parser.push_text(&text.xml10_content()?)?,
            Event::GeneralRef(node) => parser.reference(&node)?,
            Event::DocType(_doctype) => return Err(S3ListingError::Dtd),
            Event::Decl(_decl) => {
                if parser.root_seen || parser.depth != 0 {
                    return Err(S3ListingError::Trailing);
                }
            }
            Event::PI(_target) | Event::Comment(_comment) => {}
            Event::Eof => break,
        }
    }
    parser.finish()
}

pub fn parse_size(text: Option<&str>) -> Result<i64, S3ListingError> {
    let Some(raw) = text.filter(|raw| !raw.is_empty()) else {
        return Ok(0);
    };
    parse_python_int(raw).map_err(|_error| S3ListingError::Size {
        text: raw.to_owned(),
    })
}

pub fn parse_created_at(text: Option<&str>) -> Result<i64, S3ListingError> {
    let Some(raw) = text.filter(|raw| !raw.is_empty()) else {
        return Ok(0);
    };
    created_at(raw).ok_or_else(|| S3ListingError::Timestamp {
        text: raw.to_owned(),
    })
}

// LiteLLM replaces each Z with +00:00 and calls fromisoformat, then truncates the timestamp with int()
fn created_at(raw: &str) -> Option<i64> {
    if raw.contains('z') {
        return None;
    }
    let normalized = raw.replace('Z', "+00:00").replace(',', ".");
    let moment = DateTime::parse_from_rfc3339(&normalized)
        .or_else(|_error| DateTime::parse_from_rfc3339(&format!("{normalized}+00:00")))
        .ok()?;
    if !(1..=9999).contains(&moment.year()) || moment.timestamp_subsec_nanos() >= 1_000_000_000 {
        return None;
    }
    let stamp = moment.timestamp();
    if moment.timestamp_subsec_nanos() > 0 && stamp < 0 {
        return stamp.checked_add(1);
    }
    Some(stamp)
}
