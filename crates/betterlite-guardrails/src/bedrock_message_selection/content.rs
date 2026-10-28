use super::wire::{Content, Message, ModelResponse, Part, Role};
use crate::bedrock_records::{
    ContentItem, QualifiedTextBlock, Qualifier, Request, Source, TextContent,
};
use std::collections::BTreeMap;

#[must_use]
pub fn extract(message: &Message) -> Option<Vec<QualifiedTextBlock>> {
    match message.content.as_ref()? {
        Content::Text(text) => Some(vec![QualifiedTextBlock {
            text: text.clone(),
            qualifier: None,
        }]),
        Content::List(items) => Some(
            items
                .iter()
                .filter_map(|item| match item {
                    Part::Text(text) => Some(QualifiedTextBlock {
                        text: text.clone(),
                        qualifier: None,
                    }),
                    Part::Block(block) => block.text.as_ref().map(|text| QualifiedTextBlock {
                        text: text.clone(),
                        qualifier: match block.kind.as_deref() {
                            Some("grounding_source") => Some(Qualifier::GroundingSource),
                            Some("query") => Some(Qualifier::Query),
                            Some(_) | None => None,
                        },
                    }),
                    Part::Other(_) => None,
                })
                .collect(),
        ),
    }
}

#[must_use]
pub fn build_item(block: &QualifiedTextBlock) -> ContentItem {
    let qualifiers = block.qualifier.map(|qualifier| vec![qualifier]);
    ContentItem {
        text: Some(TextContent {
            text: Some(block.text.clone()),
            qualifiers,
            extra: BTreeMap::new(),
        }),
        extra: BTreeMap::new(),
    }
}

#[must_use]
pub fn input(messages: Option<&[Message]>) -> Request {
    let content = messages.map(|items| {
        items
            .iter()
            .filter_map(extract)
            .flatten()
            .map(|mut block| {
                block.qualifier = None;
                build_item(&block)
            })
            .collect()
    });
    Request {
        source: Some(Source::Input),
        content,
        extra: BTreeMap::new(),
    }
}

#[must_use]
pub fn grounding(
    messages: Option<&[Message]>,
    contextual_fallback: bool,
) -> Vec<QualifiedTextBlock> {
    let available = messages.unwrap_or_default();
    let blocks: Vec<_> = available
        .iter()
        .flat_map(|message| {
            let trusted = matches!(message.role, Some(Role::System | Role::Developer));
            extract(message).into_iter().flatten().filter(move |block| {
                block.qualifier == Some(Qualifier::Query)
                    || trusted && block.qualifier == Some(Qualifier::GroundingSource)
            })
        })
        .collect();
    if !blocks.is_empty() || !contextual_fallback {
        return blocks;
    }
    derive_grounding(available)
}

fn derive_grounding(messages: &[Message]) -> Vec<QualifiedTextBlock> {
    let Some(latest) = messages
        .iter()
        .rposition(|message| message.role == Some(Role::User))
        .and_then(|index| messages.get(index))
    else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    for message in messages
        .iter()
        .filter(|message| matches!(message.role, Some(Role::System | Role::Developer)))
    {
        append_plain(&mut sources, message, Qualifier::GroundingSource);
    }
    let mut queries = Vec::new();
    append_plain(&mut queries, latest, Qualifier::Query);
    if sources.is_empty() || queries.is_empty() {
        return Vec::new();
    }
    sources.extend(queries);
    sources
}

fn append_plain(output: &mut Vec<QualifiedTextBlock>, message: &Message, qualifier: Qualifier) {
    if let Some(blocks) = extract(message) {
        output.extend(
            blocks
                .into_iter()
                .filter(|block| !block.text.is_empty())
                .map(|block| QualifiedTextBlock {
                    text: block.text,
                    qualifier: Some(qualifier),
                }),
        );
    }
}

#[must_use]
pub fn output(
    response: Option<&ModelResponse>,
    messages: Option<&[Message]>,
    contextual_fallback: bool,
) -> Request {
    let blocks = grounding(messages, contextual_fallback);
    let qualifier = (!blocks.is_empty()).then_some(Qualifier::GuardContent);
    let mut content: Vec<_> = blocks.iter().map(build_item).collect();
    let choices = response
        .and_then(|model| model.choices.as_ref())
        .into_iter()
        .flatten();
    content.extend(
        choices.filter_map(|choice| match choice.message.as_ref()?.content.as_ref()? {
            Content::Text(text) if !text.is_empty() => Some(build_item(&QualifiedTextBlock {
                text: text.clone(),
                qualifier,
            })),
            Content::Text(_) | Content::List(_) => None,
        }),
    );
    Request {
        source: Some(Source::Output),
        content: Some(content),
        extra: BTreeMap::new(),
    }
}

#[must_use]
pub fn convert(
    source: Source,
    messages: Option<&[Message]>,
    response: Option<&ModelResponse>,
    contextual_fallback: bool,
) -> Request {
    match source {
        Source::Input => input(messages),
        Source::Output => output(response, messages, contextual_fallback),
    }
}
