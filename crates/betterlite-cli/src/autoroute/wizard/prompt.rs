use std::io::{self, Write as _};

use inquire::{Confirm, MultiSelect, Select, Text};

use super::Prompter;
use super::error::WizardError;

pub struct InquirePrompter;

impl Prompter for InquirePrompter {
    #[cfg_attr(test, mutants::skip)]
    fn announce(&mut self, message: &str) -> Result<(), WizardError> {
        let mut stdout = io::stdout();
        writeln!(stdout, "{message}").map_err(WizardError::Output)
    }

    #[cfg_attr(test, mutants::skip)]
    fn select_one(&mut self, message: &str, choices: &[String]) -> Result<String, WizardError> {
        Select::new(message, choices.to_vec())
            .prompt()
            .map_err(WizardError::Prompt)
    }

    #[cfg_attr(test, mutants::skip)]
    fn select_many(
        &mut self,
        message: &str,
        choices: &[String],
    ) -> Result<Vec<String>, WizardError> {
        MultiSelect::new(message, choices.to_vec())
            .prompt()
            .map_err(WizardError::Prompt)
    }

    #[cfg_attr(test, mutants::skip)]
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool, WizardError> {
        Confirm::new(message)
            .with_default(default)
            .prompt()
            .map_err(WizardError::Prompt)
    }

    #[cfg_attr(test, mutants::skip)]
    fn prompt_keywords(&mut self, message: &str, default: &str) -> Result<String, WizardError> {
        Text::new(message)
            .with_default(default)
            .prompt()
            .map_err(WizardError::Prompt)
    }
}
