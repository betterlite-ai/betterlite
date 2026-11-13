use std::collections::BTreeSet;

use axum::http::Method;
use axum::http::header::AUTHORIZATION;
use betterlite_roi::records::{ROIPullCommit, ROIPullEvidence, ROIPullFile};
use betterlite_roi::settings::normalize_email;
use chrono::NaiveDate;
use futures_util::TryStreamExt as _;
use serde::de::Error as _;

use crate::roi_github::adapters::rest_commit_evidence;
use crate::roi_github::adapters::{graphql_commit_evidence, pull_file_evidence};
use crate::roi_github::client::GitHub;
use crate::roi_github::error::SourceError;
use crate::roi_github::models::{
    GRAPHQL_QUERY, GitHubPullListItem, GraphQlConnection, GraphQlNode, GraphQlPayload,
    GraphQlResponse, GraphQlVariables, PullDetail, PullFile, RestCommit,
};
use crate::roi_github::pages::pages;
use crate::roi_github::profile_email::profile_email;
use crate::roi_github::transport::Transport;

const PULL_PAGES: u32 = 10_000;
const FILE_PAGES: u32 = 30;
const REST_COMMIT_PAGES: u32 = 3;
const GRAPHQL_COMMIT_PAGES: u32 = 100;
const PULL_DETAIL_ERROR: &str = "GitHub returned unexpected pull request details.";
const COMMIT_READ_ERROR: &str =
    "GitHub could not read commit metadata. Check repository permissions and API compatibility.";
const COMMIT_PARSE_ERROR: &str = "GitHub returned unexpected commit metadata.";

#[derive(Debug)]
pub struct CommitMetadata {
    pub commits: Vec<ROIPullCommit>,
    pub authors: Vec<(String, String)>,
    pub count: i64,
}

pub async fn pulls<T>(
    github: &GitHub<T>,
    repo: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<GitHubPullListItem>, SourceError>
where
    T: Transport,
{
    let start_day = start.to_string();
    let end_day = end.to_string();
    let params = [
        ("state".to_owned(), "closed".to_owned()),
        ("sort".to_owned(), "updated".to_owned()),
        ("direction".to_owned(), "desc".to_owned()),
    ];
    let path = format!("repos/{repo}/pulls");
    let mut stream = Box::pin(pages::<GitHubPullListItem, T>(
        github, &path, &params, PULL_PAGES,
    ));
    let mut merged = Vec::new();
    while let Some(page) = stream.try_next().await? {
        let stop = page.last().is_some_and(|pull| {
            pull.updated_at.get(..10).unwrap_or(&pull.updated_at) < start_day.as_str()
        });
        for pull in page {
            let within = pull.merged_at.as_deref().is_some_and(|merged_at| {
                let day = merged_at.get(..10).unwrap_or(merged_at);
                start_day.as_str() <= day && day <= end_day.as_str()
            });
            if within {
                merged.push(pull);
            }
        }
        if stop {
            break;
        }
    }
    Ok(merged)
}

pub async fn evidence<T>(
    github: &GitHub<T>,
    repo: &str,
    pull: &GitHubPullListItem,
) -> Result<ROIPullEvidence, SourceError>
where
    T: Transport,
{
    let base = format!("repos/{repo}/pulls/{}", pull.number);
    let (detail, files) = pull_snapshot(github, &base).await?;
    let user = detail.user.as_ref().and_then(|user| user.login.as_deref());
    let login = user
        .filter(|login| !login.is_empty())
        .unwrap_or("deleted-user");
    let profile = profile_email(github, login, "").await;
    let metadata = commit_metadata(github, repo, pull.number, &detail).await?;
    let (commit_emails, emails) = identity_emails(&profile, &metadata.authors, login);
    let changed_files = detail
        .changed_files
        .unwrap_or_else(|| i64::try_from(files.len()).unwrap_or(i64::MAX));
    let incomplete_metadata = usize::try_from(changed_files).ok() != Some(files.len())
        || usize::try_from(metadata.count).ok() != Some(metadata.commits.len());
    Ok(ROIPullEvidence {
        repo: repo.to_owned(),
        number: detail.number,
        title: detail.title,
        body: detail.body.unwrap_or_default(),
        url: detail.html_url,
        login: login.to_owned(),
        emails,
        profile_email: profile,
        commit_emails: Some(commit_emails),
        merged_at: detail.merged_at,
        head_sha: detail.head.sha,
        additions: detail.additions,
        deletions: detail.deletions,
        changed_files,
        files,
        commits: metadata.commits,
        commit_count: metadata.count,
        incomplete_metadata,
    })
}

async fn pull_snapshot<T>(
    github: &GitHub<T>,
    base: &str,
) -> Result<(PullDetail, Vec<ROIPullFile>), SourceError>
where
    T: Transport,
{
    let response = github.request(Method::GET, base, &[], None).await?;
    let detail = serde_json::from_slice(&response.body)
        .map_err(|source| parse_error(PULL_DETAIL_ERROR, source))?;
    let files_path = format!("{base}/files");
    let collected: Vec<Vec<PullFile>> = pages::<PullFile, T>(github, &files_path, &[], FILE_PAGES)
        .try_collect()
        .await?;
    let files = collected.iter().flatten().map(pull_file_evidence).collect();
    Ok((detail, files))
}

fn identity_emails(
    profile: &str,
    authors: &[(String, String)],
    login: &str,
) -> (Vec<String>, Vec<String>) {
    let folded = caseless::default_case_fold_str(login);
    let commit_emails: BTreeSet<String> = authors
        .iter()
        .filter(|(author, _)| caseless::default_case_fold_str(author) == folded)
        .map(|(_, address)| normalize_email(address))
        .collect();
    let mut emails = commit_emails.clone();
    emails.retain(|address| !address.is_empty());
    if !profile.is_empty() {
        emails.insert(profile.to_owned());
    }
    (
        commit_emails.into_iter().collect(),
        emails.into_iter().collect(),
    )
}

async fn commit_metadata<T>(
    github: &GitHub<T>,
    repo: &str,
    number: i64,
    detail: &PullDetail,
) -> Result<CommitMetadata, SourceError>
where
    T: Transport,
{
    if github.headers.contains_key(AUTHORIZATION) {
        return graphql_commits(github, repo, number, None, GRAPHQL_COMMIT_PAGES).await;
    }
    let path = format!("repos/{repo}/pulls/{number}/commits");
    rest_metadata(github, &path, detail).await
}

async fn rest_metadata<T>(
    github: &GitHub<T>,
    path: &str,
    detail: &PullDetail,
) -> Result<CommitMetadata, SourceError>
where
    T: Transport,
{
    let collected: Vec<Vec<RestCommit>> =
        pages::<RestCommit, T>(github, path, &[], REST_COMMIT_PAGES)
            .try_collect()
            .await?;
    let rest: Vec<&RestCommit> = collected.iter().flatten().collect();
    let count = detail
        .commits
        .unwrap_or_else(|| i64::try_from(rest.len()).unwrap_or(i64::MAX));
    let (commits, authors) = rest
        .iter()
        .map(|commit| (rest_commit_evidence(commit), rest_author(commit)))
        .unzip();
    Ok(CommitMetadata {
        commits,
        authors,
        count,
    })
}

fn rest_author(commit: &RestCommit) -> (String, String) {
    let identity = commit.commit.author.as_ref();
    let login = commit.author.as_ref().and_then(|user| user.login.clone());
    let email = identity
        .map(|author| author.email.clone())
        .unwrap_or_default();
    (login.unwrap_or_default(), email)
}

async fn graphql_commits<T>(
    github: &GitHub<T>,
    repo: &str,
    number: i64,
    cursor: Option<String>,
    remaining_pages: u32,
) -> Result<CommitMetadata, SourceError>
where
    T: Transport,
{
    let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
    // GitHub Enterprise serves GraphQL beside its /api/v3 REST root, so the path steps out of it
    let path = if github.api_url.ends_with("/api/v3") {
        "../graphql"
    } else {
        "graphql"
    };
    let mut commits = Vec::new();
    let mut authors = Vec::new();
    let mut next = cursor;
    for _ in 0..remaining_pages {
        let payload = GraphQlPayload {
            query: GRAPHQL_QUERY.to_owned(),
            variables: GraphQlVariables {
                owner: owner.to_owned(),
                name: name.to_owned(),
                number,
                cursor: next.clone(),
            },
        };
        let body = serde_json::to_vec(&payload)
            .map_err(|source| parse_error(COMMIT_PARSE_ERROR, source))?;
        let response = github.request(Method::POST, path, &[], Some(body)).await?;
        let parsed: GraphQlResponse = serde_json::from_slice(&response.body)
            .map_err(|source| parse_error(COMMIT_PARSE_ERROR, source))?;
        let connection = graphql_connection(&parsed)?;
        commits.extend(connection.nodes.iter().map(graphql_commit_evidence));
        authors.extend(connection.nodes.iter().map(node_author));
        if !connection.page_info.has_next_page {
            return Ok(CommitMetadata {
                commits,
                authors,
                count: connection.total_count,
            });
        }
        next.clone_from(&connection.page_info.end_cursor);
    }
    Err(SourceError::PaginationLimit)
}

fn graphql_connection(response: &GraphQlResponse) -> Result<&GraphQlConnection, SourceError> {
    let pull_request = response
        .data
        .as_ref()
        .and_then(|data| data.repository.as_ref())
        .and_then(|repository| repository.pull_request.as_ref())
        .filter(|_| response.errors.is_empty());
    // a message without a parse cause has no other SourceError variant to travel in
    let commits = pull_request.map(|request| &request.commits);
    commits.ok_or_else(|| SourceError::Pagination {
        message: COMMIT_READ_ERROR.to_owned(),
        source: serde_json::Error::custom(COMMIT_READ_ERROR),
    })
}

fn node_author(node: &GraphQlNode) -> (String, String) {
    let identity = node.commit.author.as_ref();
    let user = identity.and_then(|author| author.user.as_ref());
    let login = user.and_then(|account| account.login.clone());
    let email = identity
        .map(|author| author.email.clone())
        .unwrap_or_default();
    (login.unwrap_or_default(), email)
}

fn parse_error(message: &str, source: serde_json::Error) -> SourceError {
    SourceError::Pagination {
        message: message.to_owned(),
        source,
    }
}
mod tests;

include!("roi_evidence/tests/declarations.rs");
