use super::GitLabTemplateManager;
use super::error::ManagerError;
use super::sandbox::render_sandboxed;
use crate::arguments::json::Object;
use crate::gitlab_client::transport::GitLabTransport;

// The workspace denies multiple_inherent_impl, so construction and loading are free functions in their modules.
impl<T: GitLabTransport> GitLabTemplateManager<T> {
    pub fn render_template(
        &self,
        template_id: &str,
        variables: &Object,
    ) -> Result<String, ManagerError> {
        let Some(template) = self.prompts.get(template_id) else {
            return Err(ManagerError::NotFound(template_id.to_owned()));
        };
        Ok(render_sandboxed(template_id, &template.content, variables)?)
    }
}

mod tests;
