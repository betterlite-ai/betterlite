use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Default)]
pub struct CiCommits {
    pub render_git_commit: Option<String>,
    pub ci_commit_sha: Option<String>,
    pub circle_sha1: Option<String>,
    pub source_version: Option<String>,
    pub travis_commit: Option<String>,
    pub git_commit: Option<String>,
    pub github_sha: Option<String>,
    pub bitbucket_commit: Option<String>,
    pub build_sourceversion: Option<String>,
    pub drone_commit_sha: Option<String>,
}

// Raw text keeps a malformed value failing at logger construction, where LiteLLM fails it.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub secret_key: Option<SecretString>,
    pub public_key: Option<String>,
    pub host: Option<String>,
    pub base_url: Option<String>,
    pub debug: Option<String>,
    pub flush_interval: Option<String>,
    pub flush_at: Option<String>,
    pub timeout: Option<String>,
    pub max_retries: Option<String>,
    pub release: Option<String>,
    pub commits: CiCommits,
    pub sample_rate: Option<String>,
    pub prompt_cache_ttl: Option<String>,
    pub tracing_environment: Option<String>,
    pub mock: Option<String>,
    pub upstream_secret_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            secret_key: lookup.lookup("LANGFUSE_SECRET_KEY").map(SecretString::from),
            public_key: lookup.lookup("LANGFUSE_PUBLIC_KEY"),
            host: lookup.lookup("LANGFUSE_HOST"),
            base_url: lookup.lookup("LANGFUSE_BASE_URL"),
            debug: lookup.lookup("LANGFUSE_DEBUG"),
            flush_interval: lookup.lookup("LANGFUSE_FLUSH_INTERVAL"),
            flush_at: lookup.lookup("LANGFUSE_FLUSH_AT"),
            timeout: lookup.lookup("LANGFUSE_TIMEOUT"),
            max_retries: lookup.lookup("LANGFUSE_MAX_RETRIES"),
            release: lookup.lookup("LANGFUSE_RELEASE"),
            commits: CiCommits {
                render_git_commit: lookup.lookup("RENDER_GIT_COMMIT"),
                ci_commit_sha: lookup.lookup("CI_COMMIT_SHA"),
                circle_sha1: lookup.lookup("CIRCLE_SHA1"),
                source_version: lookup.lookup("SOURCE_VERSION"),
                travis_commit: lookup.lookup("TRAVIS_COMMIT"),
                git_commit: lookup.lookup("GIT_COMMIT"),
                github_sha: lookup.lookup("GITHUB_SHA"),
                bitbucket_commit: lookup.lookup("BITBUCKET_COMMIT"),
                build_sourceversion: lookup.lookup("BUILD_SOURCEVERSION"),
                drone_commit_sha: lookup.lookup("DRONE_COMMIT_SHA"),
            },
            sample_rate: lookup.lookup("LANGFUSE_SAMPLE_RATE"),
            prompt_cache_ttl: lookup.lookup("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS"),
            tracing_environment: lookup.lookup("LANGFUSE_TRACING_ENVIRONMENT"),
            mock: lookup.lookup("LANGFUSE_MOCK"),
            upstream_secret_key: lookup
                .lookup("UPSTREAM_LANGFUSE_SECRET_KEY")
                .map(SecretString::from),
        })
    }
}

mod tests;
