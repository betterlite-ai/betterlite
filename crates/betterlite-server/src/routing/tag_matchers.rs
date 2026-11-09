use betterlite_config::sections::tag_matchers as config;
use betterlite_router::tag_matchers::candidates::DeploymentTags;
use betterlite_router::tag_matchers::selection::TagMatcher;

mod tests;

#[must_use]
pub fn resolve(section: &config::Settings) -> TagMatcher {
    let deployments = section
        .deployments
        .iter()
        .map(|deployment| DeploymentTags {
            tags: deployment.tags.clone(),
            tag_regex: deployment.tag_regex.clone(),
        })
        .collect();
    TagMatcher::new(deployments)
}
