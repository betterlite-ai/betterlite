use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use regex::Regex;
use serde_saphyr::NonFiniteFloatPolicy;
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle, Span, SpannedEventReceiver};

use super::definition::{
    DefinitionError, PYYAML_NON_TEXT_SCALAR, RawDefinition, RawDefinitionText, RouteDefinition,
};
use super::error::RouteLoadError;

pub const MAX_ROUTES_FILE_BYTES: usize = 0x0010_0000;
// one byte past the bound, so a file at the limit reads whole while a longer one still proves TooLarge
const READ_LIMIT_BYTES: u64 = 0x0010_0001;

#[derive(Debug, thiserror::Error)]
pub enum CustomRoutesError {
    #[error("the custom routes input is over the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error("the custom routes input is not valid yaml: {source}")]
    Yaml {
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error(
        "the custom routes input must be a list of route definitions or one route definition mapping"
    )]
    InvalidFormat,
    #[error("route definition {index} is invalid: {source}")]
    Definition {
        index: usize,
        #[source]
        source: DefinitionError,
    },
}

struct PyYamlTextSpans<'sink> {
    spans: &'sink mut Vec<(Span, String)>,
    non_text: &'sink Regex,
    collections: Vec<YamlCollection>,
}

enum YamlCollection {
    Mapping {
        expecting_key: bool,
        key: Option<String>,
    },
    Sequence {
        quote_text: bool,
    },
}

impl PyYamlTextSpans<'_> {
    fn start_value(&mut self) -> bool {
        match self.collections.last_mut() {
            Some(YamlCollection::Mapping { expecting_key, key }) if !*expecting_key => {
                *expecting_key = true;
                let field = key.take();
                matches!(
                    field.as_deref(),
                    Some("route_name" | "description" | "utterances")
                )
            }
            Some(YamlCollection::Sequence { quote_text }) => *quote_text,
            Some(YamlCollection::Mapping { .. }) | None => false,
        }
    }

    fn scalar(&mut self, text: &str) -> bool {
        match self.collections.last_mut() {
            Some(YamlCollection::Mapping { expecting_key, key }) if *expecting_key => {
                *key = Some(text.to_owned());
                *expecting_key = false;
                false
            }
            Some(YamlCollection::Mapping { expecting_key, key }) => {
                *expecting_key = true;
                let field = key.take();
                matches!(
                    field.as_deref(),
                    Some("route_name" | "description" | "utterances")
                )
            }
            Some(YamlCollection::Sequence { quote_text }) => *quote_text,
            None => false,
        }
    }
}

impl<'input> SpannedEventReceiver<'input> for PyYamlTextSpans<'_> {
    fn on_event(&mut self, event: Event<'input>, span: Span) {
        match event {
            Event::MappingStart(..) => {
                let _ = self.start_value();
                self.collections.push(YamlCollection::Mapping {
                    expecting_key: true,
                    key: None,
                });
            }
            Event::SequenceStart(..) => {
                let quote_text = self.start_value();
                self.collections
                    .push(YamlCollection::Sequence { quote_text });
            }
            Event::MappingEnd => {
                let _ = self.collections.pop();
            }
            Event::SequenceEnd => {
                let _ = self.collections.pop();
            }
            Event::Scalar(text, ScalarStyle::Plain, _, None)
                if self.scalar(text.as_ref()) && !self.non_text.is_match(text.as_ref()) =>
            {
                self.spans.push((span, text.into_owned()));
            }
            Event::Scalar(text, ..) => {
                let _ = self.scalar(text.as_ref());
            }
            Event::Alias(_) | Event::UnresolvedAlias(_) => {
                let _ = self.start_value();
            }
            _ => {}
        }
    }
}

#[must_use]
fn quote_pyyaml_text_scalars(bytes: &[u8]) -> Cow<'_, [u8]> {
    let Ok(source) = std::str::from_utf8(bytes) else {
        return Cow::Borrowed(bytes);
    };
    let Ok(non_text) = Regex::new(PYYAML_NON_TEXT_SCALAR) else {
        return Cow::Borrowed(bytes);
    };
    let mut spans = Vec::new();
    let mut parser = Parser::new_from_str(source);
    if parser
        .load(
            &mut PyYamlTextSpans {
                spans: &mut spans,
                non_text: &non_text,
                collections: Vec::new(),
            },
            false,
        )
        .is_err()
    {
        return Cow::Borrowed(bytes);
    }
    if spans.is_empty() {
        return Cow::Borrowed(bytes);
    }
    rewritten_source(source, &spans).map_or_else(
        || Cow::Borrowed(bytes),
        |rewritten| Cow::Owned(rewritten.into_bytes()),
    )
}

fn rewritten_source(source: &str, spans: &[(Span, String)]) -> Option<String> {
    let mut rewritten = String::with_capacity(source.len());
    let mut copied = 0;
    for (span, text) in spans {
        let Some(range) = span.byte_range() else {
            continue;
        };
        let Some(prefix) = source
            .get(range.clone())
            .and_then(|slice| slice.strip_suffix(text.as_str()))
        else {
            continue;
        };
        let Some(start) = range.start.checked_add(prefix.len()) else {
            continue;
        };
        let Some(before) = source.get(copied..start) else {
            continue;
        };
        let Some(word) = source.get(start..range.end) else {
            continue;
        };
        rewritten.push_str(before);
        rewritten.push('\'');
        rewritten.push_str(&word.replace('\'', "''"));
        rewritten.push('\'');
        copied = range.end;
    }
    let tail = source.get(copied..)?;
    rewritten.push_str(tail);
    Some(rewritten)
}

pub fn decode(bytes: &[u8]) -> Result<Vec<RouteDefinition>, CustomRoutesError> {
    if bytes.len() > MAX_ROUTES_FILE_BYTES {
        return Err(CustomRoutesError::TooLarge {
            limit: MAX_ROUTES_FILE_BYTES,
        });
    }
    let input = quote_pyyaml_text_scalars(bytes);
    let options = serde_saphyr::options! {
        non_finite_float_policy: NonFiniteFloatPolicy::PassThrough,
    };
    let pattern = Regex::new(PYYAML_NON_TEXT_SCALAR).ok();
    // the document is probed once for its shape, then parsed again as the routes it holds
    let document: GuardrailValue =
        serde_saphyr::from_slice_with_options(input.as_ref(), options.clone()).map_err(
            |source| CustomRoutesError::Yaml {
                source: Box::new(source),
            },
        )?;
    match document {
        GuardrailValue::Array(_) => {
            let raw: Vec<RawDefinition> =
                serde_saphyr::from_slice_with_options(input.as_ref(), options.clone()).map_err(
                    |source| CustomRoutesError::Yaml {
                        source: Box::new(source),
                    },
                )?;
            let texts: Option<Vec<RawDefinitionText>> =
                serde_saphyr::from_slice_with_options(input.as_ref(), options).ok();
            definitions(raw, texts, pattern.as_ref())
        }
        GuardrailValue::Object(_) => {
            let raw: RawDefinition =
                serde_saphyr::from_slice_with_options(input.as_ref(), options.clone()).map_err(
                    |source| CustomRoutesError::Yaml {
                        source: Box::new(source),
                    },
                )?;
            let text: Option<RawDefinitionText> =
                serde_saphyr::from_slice_with_options(input.as_ref(), options).ok();
            definitions(
                vec![raw],
                text.map(|scalars| vec![scalars]),
                pattern.as_ref(),
            )
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => Err(CustomRoutesError::InvalidFormat),
    }
}

pub fn load(path: &Path) -> Result<Vec<RouteDefinition>, RouteLoadError> {
    let file = File::open(path).map_err(|source| read_error(path, source))?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|source| read_error(path, source))?;
    decode(&bytes).map_err(|source| RouteLoadError::CustomRoutes {
        path: path.to_owned(),
        source,
    })
}

fn read_error(path: &Path, source: io::Error) -> RouteLoadError {
    if source.kind() == io::ErrorKind::NotFound {
        return RouteLoadError::FileNotFound {
            path: path.to_owned(),
            source,
        };
    }
    RouteLoadError::Read {
        path: path.to_owned(),
        source,
    }
}

fn definitions(
    raw: Vec<RawDefinition>,
    texts: Option<Vec<RawDefinitionText>>,
    pattern: Option<&Regex>,
) -> Result<Vec<RouteDefinition>, CustomRoutesError> {
    let mut parsed = Vec::with_capacity(raw.len());
    let mut texts = texts.unwrap_or_default().into_iter();
    for (index, item) in raw.into_iter().enumerate() {
        let text = texts.next();
        parsed.push(
            item.with_pyyaml_scalars(text.as_ref(), pattern)
                .into_definition()
                .map_err(|source| CustomRoutesError::Definition { index, source })?,
        );
    }
    Ok(parsed)
}
