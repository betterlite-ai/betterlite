use super::support::{DEFAULT_BASE, anthropic_config};
use crate::skills::config::BaseSkillsApiConfig as _;
use crate::skills::wire::ListSkillsParams;

#[test]
fn skills_list_params_builds_the_query_in_limit_page_source_order() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let params = ListSkillsParams {
        limit: Some(5),
        page: Some("tok".to_owned()),
        source: Some("custom".to_owned()),
    };
    let (url, query) = config.transform_list_skills_request(&params, None).unwrap();
    assert_eq!(url, "https://api.anthropic.com/v1/skills");
    assert_eq!(
        query,
        vec![
            ("limit".to_owned(), "5".to_owned()),
            ("page".to_owned(), "tok".to_owned()),
            ("source".to_owned(), "custom".to_owned()),
        ]
    );
}

#[test]
fn skills_list_params_omits_absent_values() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let (_, query) = config
        .transform_list_skills_request(&ListSkillsParams::default(), None)
        .unwrap();
    assert!(query.is_empty());
}

#[test]
fn skills_list_params_omits_zero_limit_and_empty_strings() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let params = ListSkillsParams {
        limit: Some(0),
        page: Some(String::new()),
        source: Some(String::new()),
    };
    let (_, query) = config.transform_list_skills_request(&params, None).unwrap();
    assert!(query.is_empty());
}

#[test]
fn skills_list_params_keeps_a_negative_limit_and_a_page() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let params = ListSkillsParams {
        limit: Some(-1),
        page: Some("p".to_owned()),
        source: None,
    };
    let (_, query) = config.transform_list_skills_request(&params, None).unwrap();
    assert_eq!(
        query,
        vec![
            ("limit".to_owned(), "-1".to_owned()),
            ("page".to_owned(), "p".to_owned()),
        ]
    );
}

#[test]
fn skills_list_params_uses_the_call_api_base() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let (url, _) = config
        .transform_list_skills_request(
            &ListSkillsParams::default(),
            Some("https://gateway.example"),
        )
        .unwrap();
    assert_eq!(url, "https://gateway.example/v1/skills");
}
