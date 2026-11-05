use indexmap::IndexMap;

use crate::arguments::json::Object;
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::config::GitLabConfig;
use crate::gitlab_client::transport::GitLabTransport;
use error::ManagerError;
use manager_init::ClientSource;
use parse::FrontmatterParser;
use sandbox::render_sandboxed;
use template::GitLabPromptTemplate;

pub mod encoding;
pub mod error;
mod load;
pub mod manager_init;
pub mod parse;
pub mod path;
mod render;
pub mod sandbox;
pub mod template;
mod tests;

pub struct GitLabTemplateManager<T> {
    pub gitlab_client: GitLabClient<T>,
    pub prompts_path: String,
    pub frontmatter_parser: FrontmatterParser,
    pub prompts: IndexMap<String, GitLabPromptTemplate>,
}

impl<T: GitLabTransport> GitLabTemplateManager<T> {
    pub async fn new(
        config: &GitLabConfig,
        prompt_id: Option<&str>,
        ref_name: Option<&str>,
        gitlab_client: ClientSource<T>,
        frontmatter_parser: FrontmatterParser,
    ) -> Result<Self, ManagerError> {
        let mut manager = Self {
            gitlab_client: manager_init::resolve_client(config, gitlab_client, ref_name)?,
            prompts_path: manager_init::scoped_prompts_path(config),
            frontmatter_parser,
            prompts: IndexMap::new(),
        };
        if let Some(initial) = prompt_id.filter(|id| !id.is_empty()) {
            manager.load_prompt_from_gitlab(initial, None).await?;
        }
        Ok(manager)
    }

    pub async fn load_prompt_from_gitlab(
        &mut self,
        prompt_id: &str,
        ref_override: Option<&str>,
    ) -> Result<(), ManagerError> {
        let fetched = load::fetch_template(
            &self.gitlab_client,
            &self.prompts_path,
            self.frontmatter_parser,
            prompt_id,
            ref_override,
        )
        .await?;
        if let Some(template) = fetched {
            self.prompts.insert(prompt_id.to_owned(), template);
        }
        Ok(())
    }

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
