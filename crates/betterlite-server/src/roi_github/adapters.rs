use betterlite_roi::records::{ROIPullCommit, ROIPullFile};
use serde::Serialize;

use super::models::{GraphQlNode, PullFile, RepositoryItem, RestCommit};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryValue {
    pub full_name: String,
    pub visibility: String,
    pub archived: bool,
}

#[must_use]
pub fn repository_values(repositories: &[RepositoryItem]) -> Vec<RepositoryValue> {
    let mut values = Vec::with_capacity(repositories.len());
    for repository in repositories {
        values.push(repository_value(repository));
    }
    values
}

fn repository_value(repository: &RepositoryItem) -> RepositoryValue {
    let visibility = repository.visibility.clone().unwrap_or_else(|| {
        if repository.private {
            "private".to_owned()
        } else {
            "public".to_owned()
        }
    });
    RepositoryValue {
        full_name: repository.full_name.clone(),
        visibility,
        archived: repository.archived,
    }
}

#[must_use]
pub fn pull_file_evidence(file: &PullFile) -> ROIPullFile {
    ROIPullFile {
        filename: file.filename.clone(),
        status: file.status.clone(),
        additions: file.additions,
        deletions: file.deletions,
    }
}

#[must_use]
pub fn rest_commit_evidence(commit: &RestCommit) -> ROIPullCommit {
    ROIPullCommit {
        sha: commit.sha.clone(),
        message: commit.commit.message.clone(),
        additions: None,
        deletions: None,
        changed_files: None,
    }
}

#[must_use]
pub fn graphql_commit_evidence(node: &GraphQlNode) -> ROIPullCommit {
    let commit = &node.commit;
    ROIPullCommit {
        sha: commit.oid.clone(),
        message: commit.message.clone(),
        additions: Some(commit.additions),
        deletions: Some(commit.deletions),
        changed_files: Some(commit.changed_files_if_available),
    }
}
