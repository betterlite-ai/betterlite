use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::{FileAccess, LocalFiles, StagedTemp};
use secrecy::{ExposeSecret as _, SecretString};

use crate::pi::{PI_CONFIG_DIR_ENV, ProxyGet, ProxyResponse};

pub(super) const API_KEY: &str = "sk-test-key";
pub(super) const BASE_URL: &str = "https://proxy.test";

pub(super) fn api_key() -> SecretString {
    SecretString::from(API_KEY)
}

pub(super) enum GetAnswer {
    Response(u16, Vec<u8>),
    Transport(&'static str),
}

pub(super) fn body(status: u16, body: &str) -> GetAnswer {
    GetAnswer::Response(status, body.as_bytes().to_vec())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedRequest {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) timeout: Duration,
}

pub(super) struct ScriptedGet {
    answers: Mutex<VecDeque<GetAnswer>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedGet {
    pub(super) fn new(answers: Vec<GetAnswer>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub(super) fn urls(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    fn record(&self, request: RecordedRequest) {
        self.requests.lock().unwrap().push(request);
    }

    fn next_answer(&self) -> GetAnswer {
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("no scripted answer left")
    }
}

impl ProxyGet for ScriptedGet {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout: Duration,
    ) -> Result<ProxyResponse, io::Error> {
        self.record(RecordedRequest {
            url: url.to_owned(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
            timeout,
        });
        match self.next_answer() {
            GetAnswer::Response(status, body) => Ok(ProxyResponse { status, body }),
            GetAnswer::Transport(message) => Err(io::Error::other(message)),
        }
    }
}

pub(super) struct FakeEnvironment {
    pub(super) variables: BTreeMap<String, String>,
    pub(super) home: Option<PathBuf>,
}

impl FakeEnvironment {
    pub(super) fn with_home(home: &Path) -> Self {
        Self {
            variables: BTreeMap::new(),
            home: Some(home.to_owned()),
        }
    }

    pub(super) fn with_override(directory: &Path) -> Self {
        Self {
            variables: BTreeMap::from([(
                PI_CONFIG_DIR_ENV.to_owned(),
                directory.to_str().unwrap().to_owned(),
            )]),
            home: None,
        }
    }

    pub(super) fn without_home() -> Self {
        Self {
            variables: BTreeMap::new(),
            home: None,
        }
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileFault {
    None,
    CreateParents,
    CreateStaged,
    Write,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FileCall {
    CreateParents(PathBuf),
    CreateStaged {
        directory: PathBuf,
        suffix: Option<String>,
    },
    Write(PathBuf),
    Replace {
        staged: PathBuf,
        target: PathBuf,
    },
    Remove(PathBuf),
}

pub(super) struct ScriptedFiles {
    fault: FileFault,
    calls: Mutex<Vec<FileCall>>,
}

impl ScriptedFiles {
    pub(super) fn new(fault: FileFault) -> Self {
        Self {
            fault,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<FileCall> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: FileCall) {
        self.calls.lock().unwrap().push(call);
    }
}

impl FileAccess for ScriptedFiles {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
        self.record(FileCall::CreateParents(directory.to_owned()));
        if self.fault == FileFault::CreateParents {
            return Err(io::Error::other("scripted directory failure"));
        }
        LocalFiles.create_parents(directory)
    }

    fn ensure_private_directory(&self, directory: &Path) -> io::Result<()> {
        LocalFiles.ensure_private_directory(directory)
    }

    fn directory_mode(&self, directory: &Path) -> io::Result<u32> {
        LocalFiles.directory_mode(directory)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        LocalFiles.set_mode(path, mode)
    }

    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp> {
        self.record(FileCall::CreateStaged {
            directory: directory.to_owned(),
            suffix: suffix.map(str::to_owned),
        });
        if self.fault == FileFault::CreateStaged {
            return Err(io::Error::other("scripted staging failure"));
        }
        LocalFiles.create_staged(directory, suffix)
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        self.record(FileCall::Write(staged.path.clone()));
        if self.fault == FileFault::Write {
            return Err(io::Error::other("scripted write failure"));
        }
        LocalFiles.write_staged(staged, bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        self.record(FileCall::Replace {
            staged: staged.to_owned(),
            target: target.to_owned(),
        });
        if self.fault == FileFault::Replace {
            return Err(io::Error::other("scripted replace failure"));
        }
        LocalFiles.replace(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.record(FileCall::Remove(path.to_owned()));
        LocalFiles.remove(path)
    }

    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        LocalFiles.rewrite_existing(target, bytes)
    }
}

pub(super) fn read_json(path: &Path) -> JsonValue {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
