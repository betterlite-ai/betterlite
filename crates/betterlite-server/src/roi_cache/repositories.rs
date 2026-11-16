use chrono::NaiveDate;
use futures_util::future::join_all;

use crate::roi_evidence::pulls;
use crate::roi_github::client::GitHub;
use crate::roi_github::models::GitHubPullListItem;
use crate::roi_github::transport::Transport;

use super::error::ReportBlocked;

const COMPLETE_STAGE: &str = "Analysis complete";
const PARTIAL_STAGE: &str = "Analysis complete with unavailable repositories";

pub struct RepositoryPulls {
    pub repo: String,
    pub pulls: Vec<GitHubPullListItem>,
    pub unavailable: bool,
}

pub struct RepositoryBatch {
    pub queue: Vec<(String, GitHubPullListItem)>,
    pub unavailable_repos: Vec<String>,
    pub warnings: Vec<String>,
    pub stage: &'static str,
}

pub async fn read_repository<T>(
    github: &GitHub<T>,
    repo: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> RepositoryPulls
where
    T: Transport,
{
    match pulls(github, repo, start, end).await {
        Ok(found) => RepositoryPulls {
            repo: repo.to_owned(),
            pulls: found,
            unavailable: false,
        },
        Err(_) => RepositoryPulls {
            repo: repo.to_owned(),
            pulls: Vec::new(),
            unavailable: true,
        },
    }
}

fn ordered_queue(groups: Vec<RepositoryPulls>) -> Vec<(String, GitHubPullListItem)> {
    groups
        .into_iter()
        .flat_map(|group| {
            let repo = group.repo;
            group
                .pulls
                .into_iter()
                .map(move |pull| (repo.clone(), pull))
        })
        .collect()
}

fn unavailable_notice(unavailable: &[String]) -> (Vec<String>, &'static str) {
    if unavailable.is_empty() {
        return (Vec::new(), COMPLETE_STAGE);
    }
    let warning = format!(
        "Incomplete report: could not read {}. Results include only accessible repositories. Spend-per-hour figures are unavailable until all selected repositories can be read. Check repository access or run analysis again to retry.",
        unavailable.join(", ")
    );
    (vec![warning], PARTIAL_STAGE)
}

pub async fn read_repositories<T>(
    github: &GitHub<T>,
    repos: &[String],
    start: NaiveDate,
    end: NaiveDate,
) -> Result<RepositoryBatch, ReportBlocked>
where
    T: Transport,
{
    let groups = join_all(
        repos
            .iter()
            .map(|repo| read_repository(github, repo, start, end)),
    )
    .await;
    let unavailable: Vec<String> = groups
        .iter()
        .filter(|group| group.unavailable)
        .map(|group| group.repo.clone())
        .collect();
    if unavailable.len() == repos.len() {
        return Err(ReportBlocked::NoRepositoryReadable);
    }
    let queue = ordered_queue(groups);
    if !unavailable.is_empty() && queue.is_empty() {
        return Err(ReportBlocked::NoPullRequests {
            repos: unavailable.join(", "),
        });
    }
    let (warnings, stage) = unavailable_notice(&unavailable);
    Ok(RepositoryBatch {
        queue,
        unavailable_repos: unavailable,
        warnings,
        stage,
    })
}
