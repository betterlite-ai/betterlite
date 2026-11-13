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

type Prompted<T> = Result<T, WizardError>;

pub(crate) struct FakePrompter {
    pub(crate) selections: VecDeque<Vec<String>>,
    pub(crate) singles: VecDeque<String>,
    pub(crate) confirm_answers: VecDeque<bool>,
    pub(crate) keyword_answers: VecDeque<String>,
    pub(crate) announcements: Vec<String>,
    pub(crate) one_messages: Vec<String>,
    pub(crate) one_choices: Vec<Vec<String>>,
    pub(crate) many_messages: Vec<String>,
    pub(crate) many_choices: Vec<Vec<String>>,
    pub(crate) confirm_calls: Vec<(String, bool)>,
    pub(crate) keyword_calls: Vec<(String, String)>,
    pub(crate) fail: bool,
}

impl FakePrompter {
    pub(crate) fn new() -> Self {
        Self {
            selections: VecDeque::new(),
            singles: VecDeque::new(),
            confirm_answers: VecDeque::new(),
            keyword_answers: VecDeque::new(),
            announcements: Vec::new(),
            one_messages: Vec::new(),
            one_choices: Vec::new(),
            many_messages: Vec::new(),
            many_choices: Vec::new(),
            confirm_calls: Vec::new(),
            keyword_calls: Vec::new(),
            fail: false,
        }
    }

    pub(crate) fn was_prompted(&self) -> bool {
        !(self.announcements.is_empty()
            && self.one_messages.is_empty()
            && self.many_messages.is_empty()
            && self.confirm_calls.is_empty()
            && self.keyword_calls.is_empty())
    }

    fn refuse(&self) -> Prompted<()> {
        if self.fail {
            return Err(cancelled());
        }
        Ok(())
    }

    fn note_one(&mut self, message: &str, choices: &[String]) {
        self.one_messages.push(message.to_owned());
        self.one_choices.push(choices.to_vec());
    }

    fn note_many(&mut self, message: &str, choices: &[String]) {
        self.many_messages.push(message.to_owned());
        self.many_choices.push(choices.to_vec());
    }

    fn note_keywords(&mut self, message: &str, default: &str) {
        self.keyword_calls
            .push((message.to_owned(), default.to_owned()));
    }
}

fn cancelled() -> WizardError {
    WizardError::Prompt(InquireError::OperationCanceled)
}

impl Prompter for FakePrompter {
    fn announce(&mut self, message: &str) -> Prompted<()> {
        self.refuse()?;
        self.announcements.push(message.to_owned());
        Ok(())
    }

    fn select_one(&mut self, message: &str, choices: &[String]) -> Prompted<String> {
        self.refuse()?;
        self.note_one(message, choices);
        Ok(self.singles.pop_front().expect("a single pick"))
    }

    fn select_many(&mut self, message: &str, choices: &[String]) -> Prompted<Vec<String>> {
        self.refuse()?;
        self.note_many(message, choices);
        Ok(self.selections.pop_front().expect("a multi pick"))
    }

    fn confirm(&mut self, message: &str, default: bool) -> Prompted<bool> {
        self.refuse()?;
        self.confirm_calls.push((message.to_owned(), default));
        Ok(self.confirm_answers.pop_front().expect("a confirmation"))
    }

    fn prompt_keywords(&mut self, message: &str, default: &str) -> Prompted<String> {
        self.refuse()?;
        self.note_keywords(message, default);
        Ok(self.keyword_answers.pop_front().expect("a keyword answer"))
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
