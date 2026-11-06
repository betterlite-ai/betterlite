use indexmap::IndexMap;

use crate::gitlab_client::GitLabClient;
use parse::FrontmatterParser;
use template::GitLabPromptTemplate;

pub mod encoding;
pub mod error;
pub mod load;
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
