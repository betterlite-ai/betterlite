use betterlite_http::proxy::ProxiedClient;
use error::ManagerError;
use indexmap::IndexMap;
use parse::YamlParser;
use template::BitBucketPromptTemplate;

use crate::arguments::json::Object;
use crate::bitbucket_client::BitBucketClient;
use crate::bitbucket_client::config::BitBucketConfig;
use crate::dotprompt_templates::PromptManager;

pub mod error;
pub mod parse;
pub mod template;
mod tests;
pub mod yaml_basic;

#[derive(Debug)]
pub struct BitBucketTemplateManager {
    pub prompts: IndexMap<String, BitBucketPromptTemplate>,
    pub yaml_parser: YamlParser,
    pub client: BitBucketClient,
}

impl BitBucketTemplateManager {
    pub async fn new(
        config: BitBucketConfig,
        transport: ProxiedClient,
        prompt_id: Option<&str>,
    ) -> Result<Self, ManagerError> {
        let mut manager = Self {
            prompts: IndexMap::new(),
            yaml_parser: YamlParser::default(),
            client: BitBucketClient::new(config, transport)?,
        };
        if let Some(id) = prompt_id {
            manager.load_prompt_from_bitbucket(id).await?;
        }
        Ok(manager)
    }

    pub async fn load_prompt_from_bitbucket(
        &mut self,
        prompt_id: &str,
    ) -> Result<(), ManagerError> {
        let fetched = self
            .client
            .get_file_content(&format!("{prompt_id}.prompt"))
            .await
            .map_err(|source| ManagerError::Fetch {
                prompt_id: prompt_id.to_owned(),
                source,
            })?;
        // an empty file is as good as a missing one
        if let Some(content) = fetched.filter(|content| !content.is_empty()) {
            let template = parse::parse_prompt_file(&content, prompt_id, self.yaml_parser)
                .map_err(|source| ManagerError::Parse {
                    prompt_id: prompt_id.to_owned(),
                    source,
                })?;
            self.prompts.insert(prompt_id.to_owned(), template);
        }
        Ok(())
    }

    // The dotprompt manager already owns the sandboxed environment LiteLLM builds for both integrations
    pub fn render_template(
        &self,
        template_id: &str,
        variables: &Object,
    ) -> Result<String, ManagerError> {
        let template = self
            .prompts
            .get(template_id)
            .ok_or_else(|| ManagerError::NotFound {
                template_id: template_id.to_owned(),
            })?;
        let mut sandbox = PromptManager::default();
        sandbox.add_prompt(template_id, template.content.clone(), Object::new())?;
        Ok(sandbox.render(template_id, variables, None)?)
    }
}
