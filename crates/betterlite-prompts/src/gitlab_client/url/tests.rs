#![cfg(test)]

use rstest::rstest;

use super::{FileEndpoint, file_url};
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};

#[rstest]
#[case::slash("prompts/chat/hi.prompt", "prompts%2Fchat%2Fhi.prompt")]
#[case::space("a b", "a%20b")]
#[case::unreserved_stay("A-z_0.9~", "A-z_0.9~")]
#[case::plus_and_percent("a+b%", "a%2Bb%25")]
#[case::unicode("é", "%C3%A9")]
#[case::colon_and_at("g:r@h", "g%3Ar%40h")]
#[case::empty("", "")]
fn paths_are_encoded_but_for_unreserved_characters(#[case] text: &str, #[case] expected: &str) {
    let url = file_url("", "p", text, "r", FileEndpoint::Json);
    assert_eq!(
        url,
        format!("/projects/p/repository/files/{expected}?ref=r")
    );
}

#[test]
fn raw_and_json_urls_differ_only_by_the_raw_segment() {
    let raw = file_url(
        "https://h/api/v4",
        "g/r",
        "p/a.prompt",
        "v1",
        FileEndpoint::Raw,
    );
    let json = file_url(
        "https://h/api/v4",
        "g/r",
        "p/a.prompt",
        "v1",
        FileEndpoint::Json,
    );
    assert_eq!(
        raw,
        "https://h/api/v4/projects/g%2Fr/repository/files/p%2Fa.prompt/raw?ref=v1"
    );
    assert_eq!(
        json,
        "https://h/api/v4/projects/g%2Fr/repository/files/p%2Fa.prompt?ref=v1"
    );
}

#[test]
fn the_ref_is_encoded_with_no_safe_characters() {
    let url = file_url("https://h", "1", "a", "feature/x y", FileEndpoint::Json);
    assert!(url.ends_with("/a?ref=feature%2Fx%20y"));
}

#[tokio::test]
async fn a_per_call_ref_beats_the_client_ref_and_an_empty_one_is_ignored() {
    let transport =
        FakeTransport::scripted(vec![answer(200, "x"), answer(200, "x"), answer(200, "x")]);
    let client = GitLabClient::new(
        &config_from(
            r#"{"project": "group/repo", "access_token": "t", "branch": "dev", "base_url": "https://git.example/api/v4"}"#,
        ),
        transport.clone(),
    )
    .unwrap();
    client.get_file_content("a.prompt", None).await.unwrap();
    client
        .get_file_content("a.prompt", Some("v9"))
        .await
        .unwrap();
    client.get_file_content("a.prompt", Some("")).await.unwrap();
    let urls: Vec<String> = transport
        .requests()
        .into_iter()
        .map(|request| request.url)
        .collect();
    assert_eq!(
        urls,
        vec![
            "https://git.example/api/v4/projects/group%2Frepo/repository/files/a.prompt/raw?ref=dev",
            "https://git.example/api/v4/projects/group%2Frepo/repository/files/a.prompt/raw?ref=v9",
            "https://git.example/api/v4/projects/group%2Frepo/repository/files/a.prompt/raw?ref=dev",
        ]
    );
}
