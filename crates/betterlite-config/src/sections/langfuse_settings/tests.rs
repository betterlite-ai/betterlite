#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::langfuse_settings::Settings;

fn load(environment: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .expect("a document with no langfuse_settings keys still loads")
        .sections
        .langfuse_settings
}

#[test]
fn credentials_and_hosts_load_as_raw_text() {
    let settings = load(&[
        ("LANGFUSE_SECRET_KEY", "sk-secret"),
        ("LANGFUSE_PUBLIC_KEY", "pk-public"),
        ("LANGFUSE_HOST", "langfuse.internal:3000"),
        ("LANGFUSE_BASE_URL", "https://base.example"),
        ("LANGFUSE_DEBUG", " TRUE "),
    ]);

    assert_eq!(
        settings
            .secret_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk-secret")
    );
    assert_eq!(settings.public_key.as_deref(), Some("pk-public"));
    assert_eq!(settings.host.as_deref(), Some("langfuse.internal:3000"));
    assert_eq!(settings.base_url.as_deref(), Some("https://base.example"));
    assert_eq!(settings.debug.as_deref(), Some(" TRUE "));
}

#[test]
fn export_knobs_load_as_raw_text() {
    let settings = load(&[
        ("LANGFUSE_FLUSH_INTERVAL", "5"),
        ("LANGFUSE_FLUSH_AT", "64"),
        ("LANGFUSE_TIMEOUT", "not-a-number"),
        ("LANGFUSE_MAX_RETRIES", "-1"),
        ("LANGFUSE_SAMPLE_RATE", "0.5"),
        ("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "90"),
    ]);

    assert_eq!(settings.flush_interval.as_deref(), Some("5"));
    assert_eq!(settings.flush_at.as_deref(), Some("64"));
    assert_eq!(settings.timeout.as_deref(), Some("not-a-number"));
    assert_eq!(settings.max_retries.as_deref(), Some("-1"));
    assert_eq!(settings.sample_rate.as_deref(), Some("0.5"));
    assert_eq!(settings.prompt_cache_ttl.as_deref(), Some("90"));
}

#[test]
fn tracing_and_mock_variables_load_as_raw_text() {
    let settings = load(&[
        ("LANGFUSE_RELEASE", "release-9"),
        ("LANGFUSE_TRACING_ENVIRONMENT", " staging "),
        ("LANGFUSE_MOCK", "true"),
        ("UPSTREAM_LANGFUSE_SECRET_KEY", "upstream-secret"),
    ]);

    assert_eq!(settings.release.as_deref(), Some("release-9"));
    assert_eq!(settings.tracing_environment.as_deref(), Some(" staging "));
    assert_eq!(settings.mock.as_deref(), Some("true"));
    assert_eq!(
        settings
            .upstream_secret_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("upstream-secret")
    );
}

#[test]
fn the_ten_ci_commit_variables_load_including_empty_values() {
    let settings = load(&[
        ("RENDER_GIT_COMMIT", "render"),
        ("CI_COMMIT_SHA", ""),
        ("CIRCLE_SHA1", "circle"),
        ("SOURCE_VERSION", "source"),
        ("TRAVIS_COMMIT", "travis"),
        ("GIT_COMMIT", "git"),
        ("GITHUB_SHA", "github"),
        ("BITBUCKET_COMMIT", "bitbucket"),
        ("BUILD_SOURCEVERSION", "build"),
        ("DRONE_COMMIT_SHA", "drone"),
    ]);
    let commits = &settings.commits;

    assert_eq!(commits.render_git_commit.as_deref(), Some("render"));
    assert_eq!(commits.ci_commit_sha.as_deref(), Some(""));
    assert_eq!(commits.circle_sha1.as_deref(), Some("circle"));
    assert_eq!(commits.source_version.as_deref(), Some("source"));
    assert_eq!(commits.travis_commit.as_deref(), Some("travis"));
    assert_eq!(commits.git_commit.as_deref(), Some("git"));
    assert_eq!(commits.github_sha.as_deref(), Some("github"));
    assert_eq!(commits.bitbucket_commit.as_deref(), Some("bitbucket"));
    assert_eq!(commits.build_sourceversion.as_deref(), Some("build"));
    assert_eq!(commits.drone_commit_sha.as_deref(), Some("drone"));
}

#[test]
fn a_malformed_value_still_loads_and_keeps_its_text() {
    let settings = load(&[
        ("LANGFUSE_TIMEOUT", "soon"),
        ("LANGFUSE_FLUSH_AT", "many"),
        ("LANGFUSE_FLUSH_INTERVAL", "0"),
        ("LANGFUSE_MAX_RETRIES", "1.5"),
        ("LANGFUSE_SAMPLE_RATE", "half"),
        ("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "60.0"),
        ("LANGFUSE_TRACING_ENVIRONMENT", "Bad Env"),
        ("LANGFUSE_MOCK", "yes"),
    ]);

    assert_eq!(settings.timeout.as_deref(), Some("soon"));
    assert_eq!(settings.flush_at.as_deref(), Some("many"));
    assert_eq!(settings.flush_interval.as_deref(), Some("0"));
    assert_eq!(settings.max_retries.as_deref(), Some("1.5"));
    assert_eq!(settings.sample_rate.as_deref(), Some("half"));
    assert_eq!(settings.prompt_cache_ttl.as_deref(), Some("60.0"));
    assert_eq!(settings.tracing_environment.as_deref(), Some("Bad Env"));
    assert_eq!(settings.mock.as_deref(), Some("yes"));
}

#[test]
fn absent_credentials_and_export_knobs_stay_unset() {
    let settings = load(&[]);

    assert!(settings.secret_key.is_none());
    assert!(settings.public_key.is_none());
    assert!(settings.host.is_none());
    assert!(settings.base_url.is_none());
    assert!(settings.debug.is_none());
    assert!(settings.flush_interval.is_none());
    assert!(settings.flush_at.is_none());
    assert!(settings.timeout.is_none());
    assert!(settings.max_retries.is_none());
    assert!(settings.sample_rate.is_none());
    assert!(settings.prompt_cache_ttl.is_none());
    assert!(settings.tracing_environment.is_none());
    assert!(settings.mock.is_none());
    assert!(settings.upstream_secret_key.is_none());
}

#[test]
fn absent_release_variables_stay_unset() {
    let settings = load(&[]);
    let commits = &settings.commits;

    assert!(settings.release.is_none());
    assert!(commits.render_git_commit.is_none());
    assert!(commits.ci_commit_sha.is_none());
    assert!(commits.circle_sha1.is_none());
    assert!(commits.source_version.is_none());
    assert!(commits.travis_commit.is_none());
    assert!(commits.git_commit.is_none());
    assert!(commits.github_sha.is_none());
    assert!(commits.bitbucket_commit.is_none());
    assert!(commits.build_sourceversion.is_none());
    assert!(commits.drone_commit_sha.is_none());
}

#[test]
fn an_empty_variable_is_kept_as_the_empty_string() {
    let settings = load(&[
        ("LANGFUSE_SECRET_KEY", ""),
        ("LANGFUSE_PUBLIC_KEY", ""),
        ("LANGFUSE_HOST", ""),
        ("LANGFUSE_MOCK", ""),
        ("UPSTREAM_LANGFUSE_SECRET_KEY", ""),
    ]);

    assert_eq!(
        settings
            .secret_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("")
    );
    assert_eq!(settings.public_key.as_deref(), Some(""));
    assert_eq!(settings.host.as_deref(), Some(""));
    assert_eq!(settings.mock.as_deref(), Some(""));
    assert!(settings.upstream_secret_key.is_some());
}

#[test]
fn debug_output_hides_the_secret_keys() {
    let settings = load(&[
        ("LANGFUSE_SECRET_KEY", "sk-live-secret"),
        ("UPSTREAM_LANGFUSE_SECRET_KEY", "upstream-secret"),
    ]);

    let rendered = format!("{settings:?}");

    assert!(!rendered.contains("sk-live-secret"));
    assert!(!rendered.contains("upstream-secret"));
}
