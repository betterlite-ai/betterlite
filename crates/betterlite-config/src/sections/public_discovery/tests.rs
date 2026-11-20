#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::json;

use super::{
    BLOG_POSTS_URL_ENV, DEFAULT_BLOG_POSTS_URL, DEFAULT_DOCS_TITLE, DOCS_TITLE_ENV,
    LOCAL_BLOG_POSTS_ENV, Settings, UsefulLink, UsefulLinkEntry,
};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .public_discovery)
}

#[test]
fn loads_public_lists() {
    let source = "litellm_settings:\n  public_mcp_servers:\n    - alpha\n    - beta\n  public_mcp_hub_strict_whitelist: false\n  public_model_groups_links:\n    Status:\n      url: https://status.test\n      index: 2\n    Docs: https://docs.test\n";
    let settings = load(source, &[]).unwrap();
    assert_eq!(
        settings.public_mcp_servers,
        Some(vec!["alpha".to_owned(), "beta".to_owned()])
    );
    assert!(!settings.public_mcp_hub_strict);
    assert_eq!(
        settings
            .public_model_groups_links
            .keys()
            .collect::<Vec<_>>(),
        ["Status", "Docs"]
    );
    assert_eq!(
        settings.public_model_groups_links.get("Docs"),
        Some(&UsefulLink::Url("https://docs.test".to_owned()))
    );
    assert_eq!(
        settings.public_model_groups_links.get("Status"),
        Some(&UsefulLink::Entry(UsefulLinkEntry {
            url: Some(json!("https://status.test")),
            index: Some(json!(2)),
            other: IndexMap::new(),
        }))
    );
}

#[rstest]
#[case::absent("", None)]
#[case::null("litellm_settings:\n  public_mcp_servers:\n", None)]
#[case::empty("litellm_settings:\n  public_mcp_servers: []\n", Some(Vec::new()))]
fn public_mcp_servers_absent_null_and_empty(
    #[case] source: &str,
    #[case] expected: Option<Vec<String>>,
) {
    let settings = load(source, &[]).unwrap();
    assert_eq!(settings.public_mcp_servers, expected);
}

#[rstest]
#[case::absent("", true)]
#[case::null("litellm_settings:\n  public_mcp_hub_strict_whitelist:\n", true)]
#[case::false_flag("litellm_settings:\n  public_mcp_hub_strict_whitelist: false\n", false)]
fn mcp_hub_strict_whitelist_absent_null_and_false(#[case] source: &str, #[case] expected: bool) {
    let settings = load(source, &[]).unwrap();
    assert_eq!(settings.public_mcp_hub_strict, expected);
}

#[rstest]
#[case::defaults("", &[], DEFAULT_BLOG_POSTS_URL)]
#[case::environment(
    "",
    &[(BLOG_POSTS_URL_ENV, "https://env.test/rss.xml")],
    "https://env.test/rss.xml"
)]
#[case::empty_environment("", &[(BLOG_POSTS_URL_ENV, "")], DEFAULT_BLOG_POSTS_URL)]
#[case::null_key(
    "litellm_settings:\n  blog_posts_url:\n",
    &[(BLOG_POSTS_URL_ENV, "https://env.test/rss.xml")],
    "https://env.test/rss.xml"
)]
#[case::configured_key(
    "litellm_settings:\n  blog_posts_url: https://config.test/rss.xml\n",
    &[],
    "https://config.test/rss.xml"
)]
#[case::key_beats_environment(
    "litellm_settings:\n  blog_posts_url: https://config.test/rss.xml\n",
    &[(BLOG_POSTS_URL_ENV, "https://env.test/rss.xml")],
    "https://config.test/rss.xml"
)]
fn defaults_and_url_precedence(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected_url: &str,
) {
    let settings = load(source, variables).unwrap();
    assert_eq!(settings.blog_posts_url, expected_url);
    assert!(!settings.local_blog_posts);
    assert_eq!(settings.public_mcp_servers, None);
    assert!(settings.public_mcp_hub_strict);
    assert!(settings.public_model_groups_links.is_empty());
    assert_eq!(settings.docs_title, DEFAULT_DOCS_TITLE);
}

#[test]
fn resolves_environment_references_in_the_configured_url() {
    let source = "litellm_settings:\n  blog_posts_url: os.environ/BLOG_FEED\n";
    let settings = load(source, &[("BLOG_FEED", "https://reference.test/rss.xml")]).unwrap();
    assert_eq!(settings.blog_posts_url, "https://reference.test/rss.xml");
}

#[test]
fn a_missing_environment_reference_names_the_variable() {
    let source = "litellm_settings:\n  blog_posts_url: os.environ/ABSENT_FEED\n";
    let error = load(source, &[]).unwrap_err();
    assert!(error.to_string().contains("ABSENT_FEED"));
}

#[rstest]
#[case::plain_string(
    "litellm_settings:\n  public_model_groups_links:\n    Docs: https://docs.test\n",
    UsefulLink::Url("https://docs.test".to_owned())
)]
#[case::entry_with_url_and_index(
    "litellm_settings:\n  public_model_groups_links:\n    Docs:\n      url: https://docs.test\n      index: 2\n",
    UsefulLink::Entry(UsefulLinkEntry {
        url: Some(json!("https://docs.test")),
        index: Some(json!(2)),
        other: IndexMap::new(),
    })
)]
#[case::entry_with_extra_member_and_text_index(
    "litellm_settings:\n  public_model_groups_links:\n    Docs:\n      index: first\n      note: keep me\n",
    UsefulLink::Entry(UsefulLinkEntry {
        url: None,
        index: Some(json!("first")),
        other: IndexMap::from([("note".to_owned(), json!("keep me"))]),
    })
)]
fn useful_link_variants(#[case] source: &str, #[case] expected: UsefulLink) {
    let settings = load(source, &[]).unwrap();
    assert_eq!(
        settings.public_model_groups_links.get("Docs"),
        Some(&expected)
    );
}

#[test]
fn a_null_link_map_is_empty() {
    let settings = load("litellm_settings:\n  public_model_groups_links:\n", &[]).unwrap();
    assert!(settings.public_model_groups_links.is_empty());
}

#[test]
fn link_entries_keep_member_order() {
    let source = "litellm_settings:\n  public_model_groups_links:\n    Docs:\n      zeta: first\n      alpha: second\n";
    let settings = load(source, &[]).unwrap();
    let Some(UsefulLink::Entry(entry)) = settings.public_model_groups_links.get("Docs") else {
        panic!("the object link is an entry");
    };
    assert_eq!(entry.other.keys().collect::<Vec<_>>(), ["zeta", "alpha"]);
}

#[test]
fn a_link_that_is_not_a_string_or_object_is_rejected() {
    let source = "litellm_settings:\n  public_model_groups_links:\n    Docs: [1, 2]\n";
    let error = load(source, &[]).unwrap_err();
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[rstest]
#[case::default("", &[], DEFAULT_DOCS_TITLE)]
#[case::set("", &[(DOCS_TITLE_ENV, "Acme API")], "Acme API")]
#[case::empty_is_unset("", &[(DOCS_TITLE_ENV, "")], DEFAULT_DOCS_TITLE)]
fn docs_title_without_license(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings = load(source, variables).unwrap();
    assert_eq!(settings.docs_title, expected);
}

#[rstest]
#[case::unset(None, false)]
#[case::true_flag(Some("true"), true)]
#[case::spelled_true(Some("TRUE"), true)]
#[case::false_flag(Some("false"), false)]
#[case::empty_is_unset(Some(""), false)]
fn local_mode(#[case] value: Option<&str>, #[case] expected: bool) {
    let variables: Vec<(&str, &str)> = value
        .map(|text| (LOCAL_BLOG_POSTS_ENV, text))
        .into_iter()
        .collect();
    let settings = load("", &variables).unwrap();
    assert_eq!(settings.local_blog_posts, expected);
}

#[test]
fn an_unreadable_local_flag_names_its_variable() {
    let error = load("", &[(LOCAL_BLOG_POSTS_ENV, "maybe")]).unwrap_err();
    assert!(error.to_string().contains(LOCAL_BLOG_POSTS_ENV));
}

#[test]
fn default_settings_carry_litellm_defaults() {
    let settings = Settings::default();
    assert_eq!(settings.blog_posts_url, DEFAULT_BLOG_POSTS_URL);
    assert!(!settings.local_blog_posts);
    assert_eq!(settings.public_mcp_servers, None);
    assert!(settings.public_mcp_hub_strict);
    assert!(settings.public_model_groups_links.is_empty());
    assert_eq!(settings.docs_title, DEFAULT_DOCS_TITLE);
}
