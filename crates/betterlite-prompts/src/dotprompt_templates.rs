use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use minijinja::value::Serde;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

use crate::arguments::json::{Json, Object};
use loading::PromptManagerError;
use render::RenderError;
use template::{PromptTemplate, TemplateError};

mod bounds;
pub mod content;
mod conversion;
pub mod frontmatter;
pub mod loading;
mod reload;
pub mod render;
mod sandbox;
pub mod template;
pub mod validation;
pub mod version;

#[serde_as]
#[derive(Deserialize)]
struct PromptRecord {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    content: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    metadata: Object,
}

#[derive(Debug, Default)]
pub struct PromptManager {
    prompt_directory: Option<PathBuf>,
    prompts: IndexMap<String, PromptTemplate>,
}

impl PromptManager {
    pub fn new(
        prompt_id: Option<&str>,
        prompt_directory: Option<&Path>,
        prompt_data: Option<&Object>,
        prompt_file: Option<&Path>,
    ) -> Result<Self, PromptManagerError> {
        let mut manager = Self {
            prompt_directory: prompt_directory.map(Path::to_path_buf),
            prompts: IndexMap::new(),
        };

        if manager.prompt_directory.is_some() {
            manager.load_prompts_from_directory()?;
        }

        if let Some(file_path) = prompt_file {
            let Some(explicit_id) = prompt_id else {
                return Err(PromptManagerError::PromptIdRequired);
            };
            let template = loading::load_prompt_file(file_path, explicit_id)?;
            manager.prompts.insert(explicit_id.to_owned(), template);
        }

        if let Some(data) = prompt_data {
            match prompt_id {
                Some(id) if !id.is_empty() && data.contains_key("content") => {
                    manager.insert_prompt_record(id.to_owned(), data);
                }
                _ => manager.load_prompts_from_json_data(data),
            }
        }

        Ok(manager)
    }

    fn load_prompts_from_directory(&mut self) -> Result<(), PromptManagerError> {
        let Some(directory) = self.prompt_directory.as_deref() else {
            return Ok(());
        };
        if !directory.exists() {
            return Err(PromptManagerError::DirectoryMissing {
                path: directory.display().to_string(),
            });
        }

        let entries = fs::read_dir(directory).map_err(|source| PromptManagerError::Read {
            path: directory.display().to_string(),
            source,
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(OsStr::to_str) != Some("prompt") {
                continue;
            }
            let Some(entry_id) = path.file_stem().and_then(OsStr::to_str) else {
                continue;
            };
            let Ok(template) = loading::load_prompt_file(&path, entry_id) else {
                continue;
            };
            self.prompts.insert(entry_id.to_owned(), template);
        }
        Ok(())
    }

    fn insert_prompt_record(&mut self, template_id: String, prompt_info: &Object) {
        let Ok(value) = serde_json::to_value(prompt_info) else {
            return;
        };
        let Ok(record) = serde_json::from_value::<PromptRecord>(value) else {
            return;
        };
        let Ok(template) =
            PromptTemplate::new(record.content, record.metadata, Some(template_id.clone()))
        else {
            return;
        };
        self.prompts.insert(template_id, template);
    }

    #[must_use]
    pub fn get_prompt(&self, prompt_id: &str, version: Option<i64>) -> Option<&PromptTemplate> {
        if let Some(requested_version) = version {
            let versioned_id = format!("{prompt_id}.v{requested_version}");
            if let Some(template) = self.prompts.get(&versioned_id) {
                return Some(template);
            }
        }

        if let Some(template) = self.prompts.get(prompt_id) {
            return Some(template);
        }

        let base_prompt_id = version::strip_version_suffix(prompt_id)?;
        self.prompts.get(base_prompt_id)
    }

    pub fn render(
        &self,
        prompt_id: &str,
        prompt_variables: &Object,
        version: Option<i64>,
    ) -> Result<String, RenderError> {
        let Some(template) = self.get_prompt(prompt_id, version) else {
            // Python's truthiness check treats an explicit version of 0 the same as no version.
            let version_suffix = match version {
                Some(requested_version) if requested_version != 0 => {
                    format!(" (version {requested_version})")
                }
                _ => String::new(),
            };
            return Err(RenderError::NotFound {
                prompt_id: prompt_id.to_owned(),
                version_suffix,
                available: render::format_available(self.prompts.keys()),
            });
        };

        if !template.input_schema.is_empty() {
            validation::validate_input(prompt_variables, &template.input_schema)?;
        }

        bounds::check_template(&template.content)?;

        let env = sandbox::build();
        let rendered = env
            .render_str(&template.content, Serde(prompt_variables))
            .map_err(|source| RenderError::Template {
                prompt_id: prompt_id.to_owned(),
                source,
            })?;
        bounds::check_output(rendered)
    }

    pub fn reload_prompts(&mut self) -> Result<(), PromptManagerError> {
        self.prompts.clear();
        if self.prompt_directory.is_some() {
            self.load_prompts_from_directory()?;
        }
        Ok(())
    }

    pub fn add_prompt(
        &mut self,
        prompt_id: &str,
        content: String,
        metadata: Object,
    ) -> Result<(), TemplateError> {
        let template = PromptTemplate::new(content, metadata, Some(prompt_id.to_owned()))?;
        self.prompts.insert(prompt_id.to_owned(), template);
        Ok(())
    }

    pub fn load_prompts_from_json_data(&mut self, prompt_data: &Object) {
        for (template_id, prompt_info) in prompt_data
            .iter()
            .filter_map(|(key, value)| value.as_object().map(|object| (key.clone(), object)))
        {
            self.insert_prompt_record(template_id, prompt_info);
        }
    }

    pub fn prompt_file_to_json(file_path: &Path) -> Result<Object, PromptManagerError> {
        let content = fs::read_to_string(file_path).map_err(|source| PromptManagerError::Read {
            path: file_path.display().to_string(),
            source,
        })?;
        let (metadata, template_content) = frontmatter::parse_frontmatter(&content)?;
        Ok(conversion::file_json(template_content.trim(), metadata))
    }

    #[must_use]
    pub fn get_all_prompts_as_json(&self) -> Object {
        let mut result = Object::new();
        for (prompt_id, template) in &self.prompts {
            result.insert(
                prompt_id.clone(),
                Json::Object(conversion::file_json(
                    &template.content,
                    template.metadata.clone(),
                )),
            );
        }
        result
    }
}

mod tests;
