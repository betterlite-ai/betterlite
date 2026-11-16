#![cfg(test)]

mod support;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use betterlite_core::skills::store::SkillPage;
use betterlite_http::skill_archive::build_skill_archive;
use rstest::rstest;

use crate::custom_url::CustomUrls;
use crate::middleware::request_root_paths::RequestRootPath;

use super::archives::ZipArchiveBuilder;
use super::{AGENT_SKILLS_INDEX_PATH, SKILLS_INDEX_ALIAS_PATH};

use self::support::{
    BOGUS_KEY, CLOCK_START, DetailWire, Harness, IndexWire, LATER_STAMP, REAL_MANIFEST, STAMP,
    ScriptedArchives, ScriptedRepository, archive_path, get, get_request, many_skills, row, send,
    sized_archive, skill_archive, zip_upload,
};

#[rstest]
#[case::index(AGENT_SKILLS_INDEX_PATH)]
#[case::alias(SKILLS_INDEX_ALIAS_PATH)]
#[tokio::test]
async fn route_index_paths_answer_a_keyless_get(#[case] path: &str) {
    let harness = Harness::new(Vec::new());
    let reply = send(
        &harness.router(),
        get_request(path)
            .header(header::AUTHORIZATION, BOGUS_KEY)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header(header::CONTENT_TYPE), "application/json");
    assert!(!harness.repository.calls().is_empty());
}

#[rstest]
#[case::post("POST")]
#[case::put("PUT")]
#[case::patch("PATCH")]
#[case::delete("DELETE")]
#[tokio::test]
async fn route_index_takes_only_get(#[case] method: &str) {
    let request = Request::builder()
        .method(method)
        .uri(AGENT_SKILLS_INDEX_PATH)
        .body(Body::empty())
        .unwrap();
    let reply = send(&Harness::new(Vec::new()).router(), request).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn route_alias_serves_the_same_index() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness.archives.answering(
        b"upload-1",
        skill_archive(Some("Weather"), None, "sha256:one"),
    );
    let router = harness.router();
    let primary = get(&router, AGENT_SKILLS_INDEX_PATH).await;
    let alias = get(&router, SKILLS_INDEX_ALIAS_PATH).await;
    assert_eq!(primary.status, StatusCode::OK);
    assert_eq!(alias.status, StatusCode::OK);
    assert_eq!(primary.body, alias.body);
}

#[rstest]
#[case::index(AGENT_SKILLS_INDEX_PATH)]
#[case::alias(SKILLS_INDEX_ALIAS_PATH)]
#[case::archive("/v1/skills/skill-1/archive")]
#[case::unknown_archive("/v1/skills/missing/archive")]
#[tokio::test]
async fn route_disabled_proxy_answers_not_found_without_a_lookup(#[case] path: &str) {
    let harness = Harness::disabled(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let reply = get(&harness.router(), path).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let body: DetailWire = reply.json();
    assert_eq!(body.detail, "Not Found");
    assert!(harness.repository.calls().is_empty());
    assert!(harness.archives.uploads().is_empty());
}

#[tokio::test]
async fn index_lists_installable_skills_oldest_first() {
    let mut oldest = row("skill-oldest", Some(b"upload-oldest"), Some(STAMP));
    oldest.display_title = Some("Oldest".to_owned());
    let mut middle = row("skill-middle", Some(b"upload-middle"), Some(STAMP));
    middle.display_title = Some("Middle".to_owned());
    let mut newest = row("skill-newest", Some(b"upload-newest"), Some(STAMP));
    newest.display_title = Some("Newest".to_owned());
    let harness = Harness::new(vec![newest, middle, oldest]);
    harness
        .archives
        .answering(b"upload-oldest", skill_archive(None, None, "sha256:oldest"));
    harness
        .archives
        .answering(b"upload-middle", skill_archive(None, None, "sha256:middle"));
    harness
        .archives
        .answering(b"upload-newest", skill_archive(None, None, "sha256:newest"));
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    let names: Vec<&str> = index
        .skills
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["oldest", "middle", "newest"]);
    let digests: Vec<&str> = index
        .skills
        .iter()
        .map(|entry| entry.digest.as_str())
        .collect();
    assert_eq!(digests, ["sha256:oldest", "sha256:middle", "sha256:newest"]);
}

#[tokio::test]
async fn index_entry_carries_name_type_description_url_and_digest() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness.archives.answering(
        b"upload-1",
        skill_archive(Some("Weather"), Some("Reads the weather"), "sha256:abc"),
    );
    let reply = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await;
    let body = reply.text();
    assert!(body.starts_with(
        "{\"$schema\":\"https://schemas.agentskills.io/discovery/0.2.0/schema.json\",\"skills\":["
    ));
    let index: IndexWire = reply.json();
    assert_eq!(
        index.discovery_schema,
        "https://schemas.agentskills.io/discovery/0.2.0/schema.json"
    );
    assert_eq!(index.skills.len(), 1);
    let entry = &index.skills[0];
    assert_eq!(entry.name, "weather");
    assert_eq!(entry.entry_type, "archive");
    assert_eq!(entry.description, "Reads the weather");
    assert_eq!(entry.url, "http://proxy.test/v1/skills/skill-1/archive");
    assert_eq!(entry.digest, "sha256:abc");
}

#[tokio::test]
async fn index_with_no_skills_is_an_empty_list() {
    let harness = Harness::new(Vec::new());
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert!(index.skills.is_empty());
    assert_eq!(
        index.discovery_schema,
        "https://schemas.agentskills.io/discovery/0.2.0/schema.json"
    );
}

#[tokio::test]
async fn index_skips_skills_without_a_file_or_a_bad_archive() {
    let harness = Harness::new(vec![
        row("skill-good", Some(b"upload-good"), Some(STAMP)),
        row("skill-no-file", None, Some(STAMP)),
        row("skill-bad", Some(b"upload-bad"), Some(STAMP)),
    ]);
    harness
        .archives
        .answering(b"upload-good", skill_archive(None, None, "sha256:good"));
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert_eq!(index.skills.len(), 1);
    assert_eq!(index.skills[0].name, "skill-good");
    assert_eq!(
        harness.archives.uploads(),
        vec![b"upload-bad".to_vec(), b"upload-good".to_vec()]
    );
}

#[tokio::test]
async fn index_reads_one_page_of_1000_without_an_owner_filter() {
    let harness = Harness::new(Vec::new());
    get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await;
    assert_eq!(
        harness.repository.page_calls(),
        vec![(
            SkillPage {
                limit: 1_000,
                offset: 0
            },
            None
        )]
    );
}

#[tokio::test]
async fn index_surfaces_a_list_failure_as_a_server_error() {
    let harness = Harness::with_repository(ScriptedRepository::unavailable());
    let reply = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.text(), "Internal Server Error");
}

#[tokio::test]
async fn index_url_follows_the_configured_proxy_base_url() {
    let mut harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness.custom_urls = CustomUrls {
        proxy_base_url: Some("https://public.test/base".to_owned()),
    };
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert_eq!(
        index.skills[0].url,
        "https://public.test/base/v1/skills/skill-1/archive"
    );
}

#[tokio::test]
async fn index_url_keeps_the_request_root_path() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let request = get_request(AGENT_SKILLS_INDEX_PATH)
        .extension(RequestRootPath("/tenant-a".to_owned()))
        .body(Body::empty())
        .unwrap();
    let reply = send(&harness.router(), request).await;
    let index: IndexWire = reply.json();
    assert_eq!(
        index.skills[0].url,
        "http://proxy.test/tenant-a/v1/skills/skill-1/archive"
    );
}

#[tokio::test]
async fn index_url_uses_the_request_authority() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let request = Request::builder()
        .method("GET")
        .uri("https://gateway.test:8443/.well-known/agent-skills/index.json")
        .body(Body::empty())
        .unwrap();
    let reply = send(&harness.router(), request).await;
    let index: IndexWire = reply.json();
    assert_eq!(
        index.skills[0].url,
        "https://gateway.test:8443/v1/skills/skill-1/archive"
    );
}

#[rstest]
#[case::spaces("Weather Report", "weather-report")]
#[case::separator_runs("a///b  c", "a-b-c")]
#[case::edges("--Hello--", "hello")]
#[case::digits("V2 Beta", "v2-beta")]
#[case::non_ascii("café au lait", "caf-au-lait")]
#[tokio::test]
async fn name_slugifies_the_declared_name(#[case] declared: &str, #[case] expected: &str) {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness.archives.answering(
        b"upload-1",
        skill_archive(Some(declared), None, "sha256:one"),
    );
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert_eq!(index.skills[0].name, expected);
}

#[tokio::test]
async fn name_falls_back_from_the_declared_name_to_the_title_then_the_id() {
    let mut titled = row("skill-titled", Some(b"upload-titled"), Some(STAMP));
    titled.display_title = Some("Titled Thing".to_owned());
    let plain = row("skill-id-only", Some(b"upload-plain"), Some(STAMP));
    // the id has no slug characters at all, so this skill falls all the way back to the fallback name
    let nothing = row("???", Some(b"upload-nothing"), Some(STAMP));
    let harness = Harness::new(vec![nothing, plain, titled]);
    harness
        .archives
        .answering(b"upload-titled", skill_archive(None, None, "sha256:titled"));
    harness
        .archives
        .answering(b"upload-plain", skill_archive(None, None, "sha256:plain"));
    harness.archives.answering(
        b"upload-nothing",
        skill_archive(None, None, "sha256:nothing"),
    );
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    let names: Vec<&str> = index
        .skills
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["titled-thing", "skill-id-only", "skill"]);
}

#[tokio::test]
async fn name_is_cut_to_64_characters_and_a_trailing_separator_is_trimmed() {
    let long = "a".repeat(70);
    let separated = format!("{}-bb", "x".repeat(63));
    let harness = Harness::new(vec![
        row("skill-separated", Some(b"upload-separated"), Some(STAMP)),
        row("skill-long", Some(b"upload-long"), Some(STAMP)),
    ]);
    harness.archives.answering(
        b"upload-separated",
        skill_archive(Some(separated.as_str()), None, "sha256:separated"),
    );
    harness.archives.answering(
        b"upload-long",
        skill_archive(Some(long.as_str()), None, "sha256:long"),
    );
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert_eq!(index.skills[0].name, "a".repeat(64));
    assert_eq!(index.skills[1].name, "x".repeat(63));
}

#[tokio::test]
async fn name_duplicates_get_ordinals_in_oldest_first_order() {
    let oldest = row("skill-oldest", Some(b"upload-oldest"), Some(STAMP));
    let middle = row("skill-middle", Some(b"upload-middle"), Some(STAMP));
    let newest = row("skill-newest", Some(b"upload-newest"), Some(STAMP));
    let harness = Harness::new(vec![newest, middle, oldest]);
    harness.archives.answering(
        b"upload-oldest",
        skill_archive(Some("Weather"), None, "sha256:oldest"),
    );
    harness.archives.answering(
        b"upload-middle",
        skill_archive(Some("Weather"), None, "sha256:middle"),
    );
    harness.archives.answering(
        b"upload-newest",
        skill_archive(Some("Weather"), None, "sha256:newest"),
    );
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    let names: Vec<&str> = index
        .skills
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["weather", "weather-2", "weather-3"]);
    let digests: Vec<&str> = index
        .skills
        .iter()
        .map(|entry| entry.digest.as_str())
        .collect();
    assert_eq!(digests, ["sha256:oldest", "sha256:middle", "sha256:newest"]);
}

#[tokio::test]
async fn name_ordinal_shortens_a_64_character_base_name() {
    let base = "b".repeat(64);
    let harness = Harness::new(vec![
        row("skill-second", Some(b"upload-second"), Some(STAMP)),
        row("skill-first", Some(b"upload-first"), Some(STAMP)),
    ]);
    harness.archives.answering(
        b"upload-second",
        skill_archive(Some(base.as_str()), None, "sha256:second"),
    );
    harness.archives.answering(
        b"upload-first",
        skill_archive(Some(base.as_str()), None, "sha256:first"),
    );
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert_eq!(index.skills[0].name, "b".repeat(64));
    assert_eq!(index.skills[1].name, format!("{}-2", "b".repeat(62)));
}

#[tokio::test]
async fn name_ordinal_trims_a_separator_left_by_its_own_cut() {
    let base = format!("{}-xy", "c".repeat(61));
    let harness = Harness::new(vec![
        row("skill-second", Some(b"upload-second"), Some(STAMP)),
        row("skill-first", Some(b"upload-first"), Some(STAMP)),
    ]);
    harness.archives.answering(
        b"upload-second",
        skill_archive(Some(base.as_str()), None, "sha256:second"),
    );
    harness.archives.answering(
        b"upload-first",
        skill_archive(Some(base.as_str()), None, "sha256:first"),
    );
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert_eq!(index.skills[0].name, base);
    assert_eq!(index.skills[1].name, format!("{}-2", "c".repeat(61)));
}

#[tokio::test]
async fn name_description_falls_through_declared_stored_and_title_to_the_unique_name() {
    let mut declared = row("skill-declared", Some(b"upload-declared"), Some(STAMP));
    declared.description = Some("Stored text".to_owned());
    let mut stored = row("skill-stored", Some(b"upload-stored"), Some(STAMP));
    stored.description = Some("  Stored text  ".to_owned());
    let mut titled = row("skill-titled", Some(b"upload-titled"), Some(STAMP));
    titled.display_title = Some("Title text".to_owned());
    let plain = row("skill-plain", Some(b"upload-plain"), Some(STAMP));
    let harness = Harness::new(vec![plain, titled, stored, declared]);
    harness.archives.answering(
        b"upload-declared",
        skill_archive(
            Some("declared-skill"),
            Some("  Declared text  "),
            "sha256:declared",
        ),
    );
    harness.archives.answering(
        b"upload-stored",
        skill_archive(Some("stored-skill"), Some("   "), "sha256:stored"),
    );
    harness.archives.answering(
        b"upload-titled",
        skill_archive(Some("title-skill"), None, "sha256:titled"),
    );
    harness
        .archives
        .answering(b"upload-plain", skill_archive(None, None, "sha256:plain"));
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    let descriptions: Vec<&str> = index
        .skills
        .iter()
        .map(|entry| entry.description.as_str())
        .collect();
    assert_eq!(
        descriptions,
        ["Declared text", "Stored text", "Title text", "skill-plain"]
    );
}

#[tokio::test]
async fn name_description_is_cut_to_1024_characters() {
    let mut skill = row("skill-long", Some(b"upload-long"), Some(STAMP));
    skill.description = Some("d".repeat(1_100));
    let harness = Harness::new(vec![skill]);
    harness
        .archives
        .answering(b"upload-long", skill_archive(None, None, "sha256:long"));
    let index: IndexWire = get(&harness.router(), AGENT_SKILLS_INDEX_PATH).await.json();
    assert_eq!(index.skills[0].description, "d".repeat(1_024));
}

#[tokio::test]
async fn archive_serves_the_zip_with_the_attachment_header() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let reply = get(&harness.router(), &archive_path("skill-1")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header(header::CONTENT_TYPE), "application/zip");
    assert_eq!(
        reply.header(header::CONTENT_DISPOSITION),
        "attachment; filename=\"skill-1.zip\""
    );
    assert_eq!(reply.body, b"repacked-zip");
}

#[tokio::test]
async fn archive_serves_a_real_upload_through_the_zip_builder() {
    let upload = zip_upload(&[("demo/SKILL.md", REAL_MANIFEST)]);
    let harness = Harness::new(vec![row(
        "skill-real",
        Some(upload.as_slice()),
        Some(STAMP),
    )]);
    let router = harness.router_with(Arc::new(ZipArchiveBuilder));
    let reply = get(&router, &archive_path("skill-real")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, build_skill_archive(&upload).unwrap().content);
}

#[tokio::test]
async fn archive_answers_not_found_for_a_skill_that_is_not_stored() {
    let harness = Harness::new(Vec::new());
    let reply = get(&harness.router(), &archive_path("missing")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let body: DetailWire = reply.json();
    assert_eq!(body.detail, "No installable skill archive for: missing");
    assert_eq!(harness.repository.find_unique_calls(), ["missing"]);
}

#[tokio::test]
async fn archive_answers_not_found_for_a_skill_without_an_upload() {
    let harness = Harness::new(vec![row("skill-empty", None, Some(STAMP))]);
    let reply = get(&harness.router(), &archive_path("skill-empty")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let body: DetailWire = reply.json();
    assert_eq!(body.detail, "No installable skill archive for: skill-empty");
    assert!(harness.archives.uploads().is_empty());
}

#[tokio::test]
async fn archive_answers_not_found_when_the_upload_is_not_a_skill_zip() {
    let harness = Harness::new(vec![row("skill-bad", Some(b"not-a-zip"), Some(STAMP))]);
    let reply = get(&harness.router(), &archive_path("skill-bad")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let body: DetailWire = reply.json();
    assert_eq!(body.detail, "No installable skill archive for: skill-bad");
    assert_eq!(harness.archives.uploads(), vec![b"not-a-zip".to_vec()]);
}

#[tokio::test]
async fn archive_treats_a_missing_database_as_no_skill() {
    let harness = Harness::with_repository(ScriptedRepository::unavailable());
    let reply = get(&harness.router(), &archive_path("skill-1")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let body: DetailWire = reply.json();
    assert_eq!(body.detail, "No installable skill archive for: skill-1");
}

#[tokio::test]
async fn archive_surfaces_a_lookup_failure_as_a_server_error() {
    let harness = Harness::with_repository(ScriptedRepository::failing_reads());
    let reply = get(&harness.router(), &archive_path("skill-1")).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.text(), "Internal Server Error");
}

#[tokio::test]
async fn cache_reuses_an_archive_for_an_hour() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let router = harness.router();
    get(&router, &archive_path("skill-1")).await;
    assert_eq!(harness.archives.uploads().len(), 1);
    harness.clock.advance_to(CLOCK_START + 3_599.0);
    let fresh = get(&router, &archive_path("skill-1")).await;
    assert_eq!(fresh.status, StatusCode::OK);
    assert_eq!(harness.archives.uploads().len(), 1);
    harness.clock.advance_to(CLOCK_START + 3_601.0);
    let stale = get(&router, &archive_path("skill-1")).await;
    assert_eq!(stale.status, StatusCode::OK);
    assert_eq!(harness.archives.uploads().len(), 2);
}

#[tokio::test]
async fn cache_serves_the_archive_route_from_what_the_index_built() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let router = harness.router();
    let index = get(&router, AGENT_SKILLS_INDEX_PATH).await;
    assert_eq!(index.status, StatusCode::OK);
    let archive = get(&router, &archive_path("skill-1")).await;
    assert_eq!(archive.status, StatusCode::OK);
    assert_eq!(archive.body, b"repacked-zip");
    assert_eq!(harness.archives.uploads().len(), 1);
}

#[tokio::test]
async fn cache_rebuilds_a_skill_without_an_updated_at() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), None)]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let router = harness.router();
    get(&router, &archive_path("skill-1")).await;
    get(&router, &archive_path("skill-1")).await;
    assert_eq!(harness.archives.uploads().len(), 2);
}

#[tokio::test]
async fn cache_rebuilds_when_the_updated_at_changes() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", skill_archive(None, None, "sha256:one"));
    let router = harness.router();
    get(&router, &archive_path("skill-1")).await;
    harness
        .repository
        .replace_row(row("skill-1", Some(b"upload-1"), Some(LATER_STAMP)));
    get(&router, &archive_path("skill-1")).await;
    assert_eq!(harness.archives.uploads().len(), 2);
}

#[tokio::test]
async fn cache_keeps_an_archive_at_the_size_limit() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", sized_archive(524_288));
    let router = harness.router();
    get(&router, &archive_path("skill-1")).await;
    get(&router, &archive_path("skill-1")).await;
    assert_eq!(harness.archives.uploads().len(), 1);
}

#[tokio::test]
async fn cache_rebuilds_an_archive_over_the_size_limit() {
    let harness = Harness::new(vec![row("skill-1", Some(b"upload-1"), Some(STAMP))]);
    harness
        .archives
        .answering(b"upload-1", sized_archive(524_289));
    let router = harness.router();
    get(&router, &archive_path("skill-1")).await;
    get(&router, &archive_path("skill-1")).await;
    assert_eq!(harness.archives.uploads().len(), 2);
}

#[tokio::test]
async fn cache_keeps_at_most_128_archives() {
    let archives = ScriptedArchives::new();
    let rows = many_skills(129, &archives);
    let mut harness = Harness::new(rows);
    harness.archives = archives;
    let router = harness.router();
    get(&router, AGENT_SKILLS_INDEX_PATH).await;
    assert_eq!(harness.archives.uploads().len(), 129);
    get(&router, AGENT_SKILLS_INDEX_PATH).await;
    // one of the 129 entries was evicted, and rebuilding it evicts the next smallest key that the walk has not reached
    let rebuilt = harness.archives.uploads().len() - 129;
    assert!((1..=2).contains(&rebuilt), "rebuilt {rebuilt} archives");
}
