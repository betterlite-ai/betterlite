use betterlite_models::identity::provider_id::ProviderId;

use super::super::json_provider::JsonProviderLookup;
use super::super::selection::ChatConfigError;
use super::super::tags::{ChatConfigTag, ConfigConstruction, ConfigSelection};
use super::harness::{CHAT_CATALOG, CallSpec, TEXT_CATALOG, World, make_world};

fn world_with_providers() -> World {
    let mut world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    world.set_json(vec![
        (ProviderId::OpenaiLike, JsonProviderLookup::Record),
        (ProviderId::Anthropic, JsonProviderLookup::MissingRecord),
        (ProviderId::Publicai, JsonProviderLookup::Record),
        (ProviderId::Aiml, JsonProviderLookup::MissingRecord),
    ]);
    world
}

pub(super) fn json_fallback_precedence() {
    let world = world_with_providers();
    json_table_precedence(&world);
    json_record(&world);
    json_missing_record(&world);
    json_absent_provider();
}

fn json_table_precedence(world: &World) {
    assert_eq!(
        world
            .select(&CallSpec::plain(ProviderId::OpenaiLike, "test-model"))
            .unwrap(),
        Some(ConfigSelection {
            tag: ChatConfigTag::OpenAiLikeChat,
            construction: ConfigConstruction::New(Vec::new()),
        }),
    );
    assert_eq!(
        world
            .select(&CallSpec::plain(ProviderId::Anthropic, "test-model"))
            .unwrap()
            .map(|selection| selection.tag),
        Some(ChatConfigTag::Anthropic),
    );
}

fn json_record(world: &World) {
    let selection = world
        .select(&CallSpec::plain(ProviderId::Publicai, "test-model"))
        .unwrap()
        .unwrap();
    assert_eq!(selection.tag, ChatConfigTag::JsonProvider);
    assert_eq!(selection.tag.class_name(), "JSONProviderConfig");
    assert_eq!(selection.construction, ConfigConstruction::New(Vec::new()));
}

fn json_missing_record(world: &World) {
    assert!(matches!(
        world.select(&CallSpec::plain(ProviderId::Aiml, "test-model")),
        Err(ChatConfigError::MissingJsonRecord {
            provider: ProviderId::Aiml
        })
    ));
}

fn json_absent_provider() {
    let world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    assert_eq!(
        world
            .select(&CallSpec::plain(ProviderId::Publicai, "test-model"))
            .unwrap(),
        None
    );
    assert_eq!(
        world
            .select(&CallSpec::plain(ProviderId::Openai, "gpt-4o"))
            .unwrap()
            .map(|selection| selection.tag),
        Some(ChatConfigTag::OpenAiGpt),
    );
}
