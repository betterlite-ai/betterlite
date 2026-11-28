use quick_xml::Error as XmlError;
use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::name::{NamespaceResolver, ResolveResult};

use super::IdpMetadataError;

const MAX_DEPTH: usize = 256;
const MAX_ELEMENTS: usize = 100_000;

#[derive(Debug, Clone, Default)]
pub(crate) struct Element {
    pub(crate) namespace: Option<Vec<u8>>,
    pub(crate) local: Vec<u8>,
    pub(crate) attributes: Vec<(Vec<u8>, String)>,
    pub(crate) text: Option<String>,
    pub(crate) children: Vec<Self>,
}

impl Element {
    pub(crate) fn attribute(&self, name: &[u8]) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _value)| key.as_slice() == name)
            .map(|(_key, value)| value.as_str())
    }

    fn is_named(&self, namespace: &str, local: &[u8]) -> bool {
        self.namespace
            .as_deref()
            .is_some_and(|bound| self.local.as_slice() == local && bound == namespace.as_bytes())
    }

    pub(crate) fn children_named<'tree>(
        &'tree self,
        namespace: &'tree str,
        local: &'tree [u8],
    ) -> impl Iterator<Item = &'tree Self> {
        self.children
            .iter()
            .filter(move |child| child.is_named(namespace, local))
    }

    pub(crate) fn find_descendant(&self, namespace: &str, local: &[u8]) -> Option<&Self> {
        if self.is_named(namespace, local) {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find_descendant(namespace, local) {
                return Some(found);
            }
        }
        None
    }

    pub(crate) fn count_descendants(&self, namespace: &str, local: &[u8]) -> usize {
        let own = usize::from(self.is_named(namespace, local));
        self.children
            .iter()
            .map(|child| child.count_descendants(namespace, local))
            .fold(own, usize::saturating_add)
    }
}

#[derive(Default)]
struct Builder {
    stack: Vec<Element>,
    root: Option<Element>,
    count: usize,
}

impl Builder {
    fn open(
        &mut self,
        element: &ResolveResult<'_>,
        start: &BytesStart<'_>,
        resolver: &NamespaceResolver,
    ) -> Result<(), IdpMetadataError> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(IdpMetadataError::DepthExceeded { limit: MAX_DEPTH });
        }
        if self.stack.is_empty() && self.root.is_some() {
            return Err(IdpMetadataError::Trailing);
        }
        self.count = self.count.saturating_add(1);
        if self.count > MAX_ELEMENTS {
            return Err(IdpMetadataError::TooManyElements {
                limit: MAX_ELEMENTS,
            });
        }
        let namespace = match element {
            ResolveResult::Bound(namespace) => Some(namespace.as_ref().to_vec()),
            ResolveResult::Unbound | ResolveResult::Unknown(_) => None,
        };
        self.stack.push(Element {
            namespace,
            local: start.local_name().as_ref().to_vec(),
            attributes: extract_attributes(start, resolver)?,
            text: None,
            children: Vec::new(),
        });
        Ok(())
    }

    fn close(&mut self) {
        let Some(element) = self.stack.pop() else {
            return;
        };
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(element);
        } else {
            self.root = Some(element);
        }
    }

    fn push_text(&mut self, content: &str) -> Result<(), IdpMetadataError> {
        let Some(top) = self.stack.last_mut() else {
            if content.chars().all(char::is_whitespace) {
                return Ok(());
            }
            return Err(IdpMetadataError::Trailing);
        };
        if top.children.is_empty() {
            top.text.get_or_insert_default().push_str(content);
        }
        Ok(())
    }

    fn reference(&mut self, node: &BytesRef<'_>) -> Result<(), IdpMetadataError> {
        if let Some(character) = node.resolve_char_ref()? {
            let mut encoded = [0_u8; 4];
            return self.push_text(character.encode_utf8(&mut encoded));
        }
        let name = node.decode()?;
        let replacement =
            resolve_predefined_entity(&name).ok_or_else(|| IdpMetadataError::Entity {
                name: name.into_owned(),
            })?;
        self.push_text(replacement)
    }

    fn finish(self) -> Result<Element, IdpMetadataError> {
        if !self.stack.is_empty() {
            return Err(IdpMetadataError::Unclosed);
        }
        self.root.ok_or(IdpMetadataError::NoRoot)
    }
}

// namespace declarations are not attributes; a prefixed name resolves or the document is refused
fn extract_attributes(
    start: &BytesStart<'_>,
    resolver: &NamespaceResolver,
) -> Result<Vec<(Vec<u8>, String)>, IdpMetadataError> {
    let mut attributes = Vec::new();
    for attribute in start.attributes() {
        let parsed = attribute.map_err(XmlError::from)?;
        let key = parsed.key.as_ref();
        if key == b"xmlns" || key.starts_with(b"xmlns:") {
            continue;
        }
        if let Some(prefix) = parsed.key.prefix()
            && let ResolveResult::Unknown(unknown) = resolver.resolve_prefix(Some(prefix), false)
        {
            return Err(unbound_prefix(&unknown));
        }
        let value = parsed.normalized_value(XmlVersion::Implicit1_0)?;
        attributes.push((key.to_vec(), value.into_owned()));
    }
    Ok(attributes)
}

fn unbound_prefix(unknown: &[u8]) -> IdpMetadataError {
    IdpMetadataError::UnboundPrefix {
        prefix: unknown.iter().map(|&byte| char::from(byte)).collect(),
    }
}

pub(crate) fn decode_document(xml: &[u8]) -> Result<Element, IdpMetadataError> {
    let mut reader = NsReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut builder = Builder::default();
    loop {
        buffer.clear();
        let event = reader.read_event_into(&mut buffer)?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(start) | Event::Empty(start) => {
                let (resolved, _local) = reader.resolver().resolve_element(start.name());
                if let ResolveResult::Unknown(prefix) = &resolved {
                    return Err(unbound_prefix(prefix));
                }
                builder.open(&resolved, &start, reader.resolver())?;
                if empty {
                    builder.close();
                }
            }
            Event::End(_end) => builder.close(),
            Event::Text(text) => builder.push_text(&text.xml10_content()?)?,
            Event::CData(text) => builder.push_text(&text.xml10_content()?)?,
            Event::GeneralRef(node) => builder.reference(&node)?,
            Event::DocType(_doctype) => return Err(IdpMetadataError::DocumentType),
            Event::Decl(_decl) => {
                if builder.root.is_some() || !builder.stack.is_empty() {
                    return Err(IdpMetadataError::Trailing);
                }
            }
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    builder.finish()
}
