use crate::records::ROIPullEvidence;

use super::models::{
    ROIEstimatorChanges, ROIEstimatorCommit, ROIEstimatorEvidence, ROIEstimatorFile,
};

#[must_use]
pub fn metadata_evidence(pull: &ROIPullEvidence) -> ROIEstimatorEvidence {
    ROIEstimatorEvidence {
        repo: pull.repo.clone(),
        number: pull.number,
        title: pull.title.clone(),
        body: pull.body.clone(),
        changes: ROIEstimatorChanges {
            additions: pull.additions,
            deletions: pull.deletions,
            files: pull.changed_files,
            commits: pull.commit_count,
        },
        files: pull
            .files
            .iter()
            .map(|file| ROIEstimatorFile {
                filename: file.filename.clone(),
                status: file.status.clone(),
                additions: file.additions,
                deletions: file.deletions,
            })
            .collect(),
        commits: pull
            .commits
            .iter()
            .map(|commit| ROIEstimatorCommit {
                sha: commit.sha.clone(),
                message: commit.message.clone(),
                additions: commit.additions,
                deletions: commit.deletions,
                changed_files: commit.changed_files,
            })
            .collect(),
    }
}
