use betterlite_http::proxy::limits::TransportLimits;
use quick_xml::NsReader;
use quick_xml::encoding::EncodingError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{NamespaceResolver, ResolveResult};

use self::state::ListingBuilder;
use super::S3ListingError;

mod state;

pub const DEFAULT_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    pub body_bytes: usize,
    pub depth: usize,
}

impl DecodeLimits {
    pub const DEFAULT: Self = Self {
        body_bytes: TransportLimits::DEFAULT.body_bytes,
        depth: DEFAULT_DEPTH,
    };
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
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

fn prefix_text(prefix: &[u8]) -> String {
    prefix.iter().map(|&byte| char::from(byte)).collect()
}

// ElementTree rejects a document whose element or attribute names use an undeclared prefix
fn check_attributes(
    resolver: &NamespaceResolver,
    start: &BytesStart<'_>,
) -> Result<(), S3ListingError> {
    for attribute in start.attributes() {
        let attribute = attribute?;
        let Some(prefix) = attribute.key.prefix() else {
            continue;
        };
        if let ResolveResult::Unknown(unknown) = resolver.resolve_prefix(Some(prefix), false) {
            return Err(S3ListingError::UnboundPrefix {
                prefix: prefix_text(unknown.0),
            });
        }
    }
    Ok(())
}

fn check_element(
    resolver: &NamespaceResolver,
    start: &BytesStart<'_>,
) -> Result<(), S3ListingError> {
    check_attributes(resolver, start)
}

pub fn decode_s3_listing(bytes: &[u8], limits: &DecodeLimits) -> Result<S3Listing, S3ListingError> {
    if bytes.len() > limits.body_bytes {
        return Err(S3ListingError::BodyTooLarge {
            limit: limits.body_bytes,
        });
    }
    let mut reader = NsReader::from_reader(bytes);
    let mut buffer = Vec::new();
    let mut builder = ListingBuilder::default();
    loop {
        let (resolved, event) = reader.read_resolved_event_into(&mut buffer)?;
        let unbound = match &resolved {
            ResolveResult::Unknown(prefix) => Some(prefix_text(prefix.0)),
            ResolveResult::Unbound | ResolveResult::Bound(_) => None,
        };
        if let Some(prefix) = unbound {
            return Err(S3ListingError::UnboundPrefix { prefix });
        }
        match event {
            Event::Start(start) => {
                check_element(reader.resolver(), &start)?;
                state::open(&mut builder, &start, limits)?;
            }
            Event::Empty(start) => {
                check_element(reader.resolver(), &start)?;
                state::open_empty(&mut builder, &start, limits)?;
            }
            Event::End(_end) => state::close(&mut builder)?,
            Event::Text(text) => {
                let content = text.xml10_content()?;
                state::text(&mut builder, &content)?;
            }
            Event::CData(text) => {
                let content = text.xml10_content()?;
                state::text(&mut builder, &content)?;
            }
            Event::GeneralRef(reference) => state::reference(&mut builder, &reference)?,
            Event::DocType(_doctype) => return Err(S3ListingError::Dtd),
            Event::Decl(_decl) => {
                if builder.root_seen {
                    return Err(S3ListingError::Trailing);
                }
            }
            Event::PI(_target) | Event::Comment(_comment) => {}
            Event::Eof => break,
        }
    }
    if !builder.root_seen {
        return Err(S3ListingError::NoRoot);
    }
    if !builder.stack.is_empty() {
        return Err(S3ListingError::Unclosed);
    }
    Ok(state::into_listing(builder))
}
