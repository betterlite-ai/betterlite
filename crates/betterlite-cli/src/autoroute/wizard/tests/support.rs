use std::cell::Cell;
use std::collections::VecDeque;
use std::io;
use std::path::Path;

use betterlite_logging::private_json::files::{FileAccess, LocalFiles, StagedTemp};
use inquire::InquireError;

use crate::autoroute::config::model::RawModelListing;
use crate::autoroute::wizard::error::{ModelDiscoveryError, WizardError};
use crate::autoroute::wizard::{ModelDiscoveryClient, Prompter};

pub(crate) struct FakeClient {
    models: Vec<RawModelListing>,
    fail: bool,
    calls: Cell<usize>,
}

impl FakeClient {
    pub(crate) fn new(models: Vec<RawModelListing>) -> Self {
        Self {
            models,
            fail: false,
            calls: Cell::new(0),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            models: Vec::new(),
            fail: true,
            calls: Cell::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl ModelDiscoveryClient for FakeClient {
    fn list_models(&self) -> Result<Vec<RawModelListing>, ModelDiscoveryError> {
        let calls = self.calls.get().checked_add(1).expect("a call counter");
        self.calls.set(calls);
        if self.fail {
            let source = serde_json::from_str::<Vec<RawModelListing>>("null")
                .expect_err("null is not a model list");
            return Err(ModelDiscoveryError::NotAList(source));
        }
        Ok(self.models.clone())
    }
}

pub(crate) enum Call {
    Announce(String),
    SelectOne {
        message: String,
        choices: Vec<String>,
    },
    SelectMany {
        message: String,
        choices: Vec<String>,
    },
    Confirm {
        message: String,
        default: bool,
    },
    Keywords {
        message: String,
        default: String,
    },
}

pub(crate) struct FakePrompter {
    pub(crate) selections: VecDeque<Vec<String>>,
    pub(crate) singles: VecDeque<String>,
    pub(crate) confirm_answers: VecDeque<bool>,
    pub(crate) keyword_answers: VecDeque<String>,
    pub(crate) calls: Vec<Call>,
    pub(crate) fail: bool,
}

impl FakePrompter {
    pub(crate) fn new() -> Self {
        Self {
            selections: VecDeque::new(),
            singles: VecDeque::new(),
            confirm_answers: VecDeque::new(),
            keyword_answers: VecDeque::new(),
            calls: Vec::new(),
            fail: false,
        }
    }

    pub(crate) fn announcements(&self) -> Vec<&str> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                Call::Announce(message) => Some(message.as_str()),
                Call::SelectOne { .. }
                | Call::SelectMany { .. }
                | Call::Confirm { .. }
                | Call::Keywords { .. } => None,
            })
            .collect()
    }

    pub(crate) fn select_many_calls(&self) -> Vec<(&str, Vec<&str>)> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                Call::SelectMany { message, choices } => Some((
                    message.as_str(),
                    choices.iter().map(String::as_str).collect(),
                )),
                Call::Announce(_)
                | Call::SelectOne { .. }
                | Call::Confirm { .. }
                | Call::Keywords { .. } => None,
            })
            .collect()
    }

    pub(crate) fn select_one_calls(&self) -> Vec<(&str, Vec<&str>)> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                Call::SelectOne { message, choices } => Some((
                    message.as_str(),
                    choices.iter().map(String::as_str).collect(),
                )),
                Call::Announce(_)
                | Call::SelectMany { .. }
                | Call::Confirm { .. }
                | Call::Keywords { .. } => None,
            })
            .collect()
    }

    pub(crate) fn confirm_calls(&self) -> Vec<(&str, bool)> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                Call::Confirm { message, default } => Some((message.as_str(), *default)),
                Call::Announce(_)
                | Call::SelectOne { .. }
                | Call::SelectMany { .. }
                | Call::Keywords { .. } => None,
            })
            .collect()
    }

    pub(crate) fn keyword_calls(&self) -> Vec<(&str, &str)> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                Call::Keywords { message, default } => Some((message.as_str(), default.as_str())),
                Call::Announce(_)
                | Call::SelectOne { .. }
                | Call::SelectMany { .. }
                | Call::Confirm { .. } => None,
            })
            .collect()
    }
}

fn cancelled() -> WizardError {
    WizardError::Prompt(InquireError::OperationCanceled)
}

impl Prompter for FakePrompter {
    fn announce(&mut self, message: &str) -> Result<(), WizardError> {
        if self.fail {
            return Err(cancelled());
        }
        self.calls.push(Call::Announce(message.to_owned()));
        Ok(())
    }

    fn select_one(&mut self, message: &str, choices: &[String]) -> Result<String, WizardError> {
        if self.fail {
            return Err(cancelled());
        }
        self.calls.push(Call::SelectOne {
            message: message.to_owned(),
            choices: choices.to_vec(),
        });
        Ok(self
            .singles
            .pop_front()
            .expect("a scripted single selection"))
    }

    fn select_many(
        &mut self,
        message: &str,
        choices: &[String],
    ) -> Result<Vec<String>, WizardError> {
        if self.fail {
            return Err(cancelled());
        }
        self.calls.push(Call::SelectMany {
            message: message.to_owned(),
            choices: choices.to_vec(),
        });
        Ok(self
            .selections
            .pop_front()
            .expect("a scripted multi selection"))
    }

    fn confirm(&mut self, message: &str, default: bool) -> Result<bool, WizardError> {
        if self.fail {
            return Err(cancelled());
        }
        self.calls.push(Call::Confirm {
            message: message.to_owned(),
            default,
        });
        Ok(self
            .confirm_answers
            .pop_front()
            .expect("a scripted confirmation"))
    }

    fn prompt_keywords(&mut self, message: &str, default: &str) -> Result<String, WizardError> {
        if self.fail {
            return Err(cancelled());
        }
        self.calls.push(Call::Keywords {
            message: message.to_owned(),
            default: default.to_owned(),
        });
        Ok(self
            .keyword_answers
            .pop_front()
            .expect("a scripted keyword answer"))
    }
}

pub(crate) struct FailingDirectory {
    inner: LocalFiles,
}

impl FailingDirectory {
    pub(crate) fn new() -> Self {
        Self { inner: LocalFiles }
    }
}

impl FileAccess for FailingDirectory {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
        self.inner.create_parents(directory)
    }

    fn ensure_private_directory(&self, _directory: &Path) -> io::Result<()> {
        Err(io::Error::other("the directory cannot be prepared"))
    }

    fn directory_mode(&self, directory: &Path) -> io::Result<u32> {
        self.inner.directory_mode(directory)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        self.inner.set_mode(path, mode)
    }

    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp> {
        self.inner.create_staged(directory, suffix)
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_staged(staged, bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        self.inner.replace(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.inner.remove(path)
    }

    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        self.inner.rewrite_existing(target, bytes)
    }
}
