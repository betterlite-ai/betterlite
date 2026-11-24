#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{
    ClaudePluginReadInput, ClaudePluginReadOperation, ClaudePluginReadResult, PluginManifestFields,
    PluginRow,
};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn row(name: &str, enabled: bool, manifest: PluginManifestFields) -> PluginRow {
    PluginRow {
        id: format!("plugin-{name}"),
        name: name.to_owned(),
        version: None,
        description: None,
        enabled,
        created_at: None,
        updated_at: None,
        created_by: None,
        manifest,
    }
}

#[test]
fn manifest_fields_keep_the_keys_the_handlers_read() {
    let manifest = Json::Object(IndexMap::from([
        (
            "source".to_owned(),
            Json::Object(IndexMap::from([("repo".to_owned(), text("org/plugin"))])),
        ),
        ("author".to_owned(), Json::Object(IndexMap::new())),
        ("homepage".to_owned(), text("https://example.com/plugin")),
        (
            "keywords".to_owned(),
            Json::List(vec![text("one"), text("two")]),
        ),
        ("category".to_owned(), text("dev")),
        ("domain".to_owned(), Json::Null),
        ("namespace".to_owned(), text("workflows")),
    ]));
    let fields = PluginManifestFields::from_manifest(manifest).expect("an object manifest decodes");
    assert_eq!(
        fields.source,
        Some(Json::Object(IndexMap::from([(
            "repo".to_owned(),
            text("org/plugin"),
        )])))
    );
    assert_eq!(fields.author, Some(Json::Object(IndexMap::new())));
    assert_eq!(fields.homepage, Some(text("https://example.com/plugin")));
    assert_eq!(
        fields.keywords,
        Some(Json::List(vec![text("one"), text("two")]))
    );
    assert_eq!(fields.category, Some(text("dev")));
    assert_eq!(fields.domain, Some(Json::Null));
    assert_eq!(fields.namespace, Some(text("workflows")));
}

#[test]
fn run_lists_the_manifest_fields_the_item_shows() {
    let manifest = PluginManifestFields {
        source: Some(Json::Object(IndexMap::from([(
            "repo".to_owned(),
            text("org/plugin"),
        )]))),
        author: None,
        homepage: Some(text("https://example.com/plugin")),
        keywords: Some(Json::List(vec![text("one"), text("two")])),
        category: Some(text("dev")),
        domain: Some(text("Productivity")),
        namespace: Some(text("workflows")),
    };
    let input = ClaudePluginReadInput {
        operation: ClaudePluginReadOperation::List {
            enabled_only: false,
        },
        caller: None,
    };
    let listed = input
        .run(vec![row("fields-plugin", true, manifest)])
        .expect("the list");
    let ClaudePluginReadResult::List(list) = listed else {
        panic!("a list request answers with the list result");
    };
    assert_eq!(list.count, 1);
    let item = list.plugins.first().expect("the visible row is listed");
    assert_eq!(item.name, "fields-plugin");
    assert_eq!(item.homepage.as_deref(), Some("https://example.com/plugin"));
    assert_eq!(
        item.keywords,
        Some(vec!["one".to_owned(), "two".to_owned()])
    );
    assert_eq!(item.category.as_deref(), Some("dev"));
    assert_eq!(item.domain.as_deref(), Some("Productivity"));
    assert_eq!(item.namespace.as_deref(), Some("workflows"));
}

#[test]
fn run_refuses_a_plugin_the_caller_cannot_see() {
    let input = ClaudePluginReadInput {
        operation: ClaudePluginReadOperation::Get {
            plugin_name: "hidden-plugin".to_owned(),
        },
        caller: None,
    };
    let hidden = row("hidden-plugin", false, PluginManifestFields::default());
    let error = input
        .run(vec![hidden])
        .expect_err("the hidden row is not granted");
    assert_eq!(
        error.to_string(),
        "Plugin 'hidden-plugin' is not granted to this key"
    );
    let visible = row("hidden-plugin", true, PluginManifestFields::default());
    let found = input
        .run(vec![visible])
        .expect("the enabled row is granted");
    let ClaudePluginReadResult::Detail(detail) = found else {
        panic!("a get request answers with the detail result");
    };
    assert_eq!(detail.name, "hidden-plugin");
}
