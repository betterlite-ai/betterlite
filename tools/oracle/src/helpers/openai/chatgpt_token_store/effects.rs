use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_openai::chatgpt::token_store::Clock;
use betterlite_openai::chatgpt::token_store::codec::AuthRecord;
use betterlite_openai::chatgpt::token_store::files::{FileAccess, LocalFiles, StoreEvent};
use rust_decimal::Decimal;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error("fixture IO failed")]
    Io(#[from] io::Error),
    #[error("fixture configuration failed")]
    Config(#[from] betterlite_config::loader::ConfigError),
    #[error("fixture store failed")]
    Store(#[from] betterlite_openai::chatgpt::token_store::StoreError),
    #[error("fixture time failed")]
    Time,
    #[error("fixture number failed")]
    Number(#[from] serde_json::Error),
}

pub(super) fn content(text: Option<&str>) -> Value {
    text.map_or(Value::Null, |value| {
        serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()))
    })
}

pub(super) struct Files {
    pub writes: Vec<Value>,
    pub events: Vec<&'static str>,
    pub reads: usize,
    pub fail_write: bool,
}

impl FileAccess for Files {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()> {
        LocalFiles.ensure_dir(path)
    }
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        self.reads = self.reads.saturating_add(1);
        LocalFiles.read(path)
    }
    fn write(&mut self, path: &Path, record: &AuthRecord) -> io::Result<()> {
        self.writes
            .push(serde_json::to_value(record).map_err(io::Error::other)?);
        let destination = if self.fail_write {
            path.join("missing")
        } else {
            path.to_path_buf()
        };
        LocalFiles.write(&destination, record)
    }
    fn report(&mut self, event: StoreEvent) {
        self.events.push(match event {
            StoreEvent::InvalidRecord => "warning",
            StoreEvent::WriteFailure => "error",
        });
    }
}

pub(super) struct ScriptClock {
    pub now: Decimal,
    pub sleeps: Vec<serde_json::Number>,
    pub path: PathBuf,
    pub replacement: Option<String>,
    pub failure: Option<io::Error>,
}

impl Clock for ScriptClock {
    fn now(&self) -> Decimal {
        self.now
    }
    fn sleep(&mut self, duration: Duration) {
        let seconds = Decimal::from(duration.as_secs())
            .saturating_add(Decimal::new(i64::from(duration.subsec_nanos()), 9));
        match seconds.to_string().parse() {
            Ok(value) => self.sleeps.push(value),
            Err(error) => self.failure = Some(io::Error::other(error)),
        }
        self.now = self.now.saturating_add(seconds);
        if let Some(replacement) = self.replacement.take() {
            self.failure = std::fs::write(&self.path, replacement).err();
        }
    }
}

pub(super) fn store(
    input: &super::Input,
    directory: &Path,
) -> Result<
    betterlite_openai::chatgpt::token_store::ChatGptTokenStore<Files, ScriptClock>,
    FixtureError,
> {
    let mut settings = betterlite_config::loader::ConfigLoader {
        lookup: std::collections::BTreeMap::<String, String>::new(),
    }
    .load("")?
    .sections
    .chatgpt_token_store;
    settings.token_dir = directory.display().to_string();
    let files = Files {
        writes: Vec::new(),
        events: Vec::new(),
        reads: 0,
        fail_write: input.fail_write,
    };
    let clock = ScriptClock {
        now: input.now,
        sleeps: Vec::new(),
        path: directory.join("auth.json"),
        failure: None,
        replacement: input.replacement.clone(),
    };
    Ok(betterlite_openai::chatgpt::token_store::ChatGptTokenStore::new(settings, files, clock)?)
}
