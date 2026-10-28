mod tests;

use crate::spec::review::{FindingOrigin, Review, ReviewVerdict};
use crate::spec::rules::Rules;

pub(crate) fn review_problems(
    review: &Review,
    task_id: &str,
    rules: &Rules,
    changed: &[String],
) -> Vec<String> {
    let mut problems = Vec::new();
    if review.task != task_id {
        problems.push(format!(
            "the review is for {}, the task is {task_id}",
            review.task
        ));
    }
    let ours = review
        .findings
        .iter()
        .any(|finding| finding.origin == FindingOrigin::Ours);
    let consistent = match review.verdict {
        ReviewVerdict::Approve => !ours,
        ReviewVerdict::Changes => ours,
    };
    if !consistent {
        problems.push(
            "approve comes with no findings of origin ours and changes with at least one"
                .to_owned(),
        );
    }
    for finding in &review.findings {
        let reviewed_rule = rules
            .rules
            .iter()
            .any(|rule| rule.id == finding.rule && rule.reviewed);
        let path = finding.location.split(':').next().unwrap_or_default();
        if !reviewed_rule
            || !changed.iter().any(|file| file == path)
            || finding.problem.trim().is_empty()
            || finding.fix.trim().is_empty()
        {
            problems.push(format!(
                "finding at {} must cite a reviewed rule, point into a changed file, and say the problem and the fix",
                finding.location
            ));
        }
    }
    problems
}
