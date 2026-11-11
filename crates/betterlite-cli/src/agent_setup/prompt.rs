use inquire::list_option::ListOption;
use inquire::validator::{ErrorMessage, Validation};
use inquire::{MultiSelect, Password, Select, Text};
use secrecy::SecretString;

use super::error::AgentSetupError;
use crate::agent_setup_profiles::Target;

pub const TARGET_LABELS: [&str; 2] = ["Claude Code (CLI)", "Codex (CLI)"];

pub trait Prompter: Send {
    fn pick_targets(
        &mut self,
        message: &str,
        defaults: &[Target],
        on_empty: &str,
    ) -> Result<Vec<Target>, AgentSetupError>;

    fn pick_model(
        &mut self,
        message: &str,
        choices: &[String],
        selected: usize,
    ) -> Result<String, AgentSetupError>;

    fn prompt_url(&mut self, message: &str, default: &str) -> Result<String, AgentSetupError>;

    fn prompt_secret(&mut self, message: &str) -> Result<SecretString, AgentSetupError>;
}

pub struct InquirePrompter;

impl Prompter for InquirePrompter {
    #[cfg_attr(test, mutants::skip)]
    fn pick_targets(
        &mut self,
        message: &str,
        defaults: &[Target],
        on_empty: &str,
    ) -> Result<Vec<Target>, AgentSetupError> {
        let checked: Vec<usize> = [Target::Claude, Target::Codex]
            .iter()
            .enumerate()
            .filter(|(_index, target)| defaults.contains(target))
            .map(|(index, _target)| index)
            .collect();
        let invalid = on_empty.to_owned();
        let validator = move |selection: &[ListOption<&&str>]| {
            if selection.is_empty() {
                Ok(Validation::Invalid(ErrorMessage::Custom(invalid.clone())))
            } else {
                Ok(Validation::Valid)
            }
        };
        let picked = MultiSelect::new(message, TARGET_LABELS.to_vec())
            .with_default(&checked)
            .with_validator(validator)
            .prompt()?;
        Ok([Target::Claude, Target::Codex]
            .into_iter()
            .zip(TARGET_LABELS)
            .filter(|(_target, label)| picked.contains(label))
            .map(|(target, _label)| target)
            .collect())
    }

    #[cfg_attr(test, mutants::skip)]
    fn pick_model(
        &mut self,
        message: &str,
        choices: &[String],
        selected: usize,
    ) -> Result<String, AgentSetupError> {
        let picked = Select::new(message, choices.to_vec())
            .with_starting_cursor(selected)
            .prompt()?;
        Ok(picked)
    }

    #[cfg_attr(test, mutants::skip)]
    fn prompt_url(&mut self, message: &str, default: &str) -> Result<String, AgentSetupError> {
        let entered = Text::new(message).with_default(default).prompt()?;
        Ok(entered)
    }

    #[cfg_attr(test, mutants::skip)]
    fn prompt_secret(&mut self, message: &str) -> Result<SecretString, AgentSetupError> {
        let entered = Password::new(message).without_confirmation().prompt()?;
        Ok(SecretString::from(entered))
    }
}
