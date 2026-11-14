use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;

use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::commands::chat::conversation::{ConversationError, ConversationStore};
use crate::commands::chat::{ChatArgs, ChatError, Message, Terminal, run_chat_command};
use crate::commands::tests::support::{Answer, TestServer, client_for};

pub(crate) struct FakeTerminal {
    input: VecDeque<String>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl FakeTerminal {
    pub(crate) fn new(lines: &[&str]) -> Self {
        Self {
            input: lines.iter().map(|line| (*line).to_owned()).collect(),
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

impl Terminal for FakeTerminal {
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Ok(self.input.pop_front())
    }

    fn write_line(&mut self, text: &str) -> io::Result<()> {
        self.stdout.push_str(text);
        self.stdout.push('\n');
        Ok(())
    }

    fn write_fragment(&mut self, text: &str) -> io::Result<()> {
        self.stdout.push_str(text);
        Ok(())
    }

    fn write_error_line(&mut self, text: &str) -> io::Result<()> {
        self.stderr.push_str(text);
        self.stderr.push('\n');
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct FakeStore {
    pub(crate) files: BTreeMap<String, Vec<Message>>,
    pub(crate) unreadable: BTreeSet<String>,
    pub(crate) unwritable: BTreeSet<String>,
    pub(crate) saved: Vec<(String, Vec<Message>)>,
}

impl ConversationStore for FakeStore {
    fn save(&mut self, filename: &str, messages: &[Message]) -> Result<(), ConversationError> {
        if self.unwritable.contains(filename) {
            return Err(ConversationError::Write {
                filename: filename.to_owned(),
                source: io::Error::other("the disk is full"),
            });
        }
        self.saved.push((filename.to_owned(), messages.to_vec()));
        let _previous = self.files.insert(filename.to_owned(), messages.to_vec());
        Ok(())
    }

    fn load(&mut self, filename: &str) -> Result<Vec<Message>, ConversationError> {
        if self.unreadable.contains(filename) {
            return Err(ConversationError::Read {
                filename: filename.to_owned(),
                source: io::Error::other("the file is not readable"),
            });
        }
        self.files
            .get(filename)
            .cloned()
            .ok_or(ConversationError::NotFound)
    }
}

pub(crate) fn chat_args(model: Option<&str>) -> ChatArgs {
    ChatArgs {
        model: model.map(str::to_owned),
        temperature: 0.7,
        max_tokens: None,
        system: None,
    }
}

pub(crate) fn run_chat(
    args: &ChatArgs,
    client: &Client,
    terminal: &mut FakeTerminal,
    store: &mut FakeStore,
) -> Result<(), ChatError> {
    run_chat_command(args, &CliContext::default(), client, terminal, store)
}

pub(crate) fn delta(content: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{content}\"}}}}]}}")
}

pub(crate) fn stream_body(fragments: &[&str]) -> String {
    let mut body = String::new();
    for fragment in fragments {
        body.push_str(&delta(fragment));
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

pub(crate) fn models_body(models: &[(&str, &str)]) -> String {
    let entries: Vec<String> = models
        .iter()
        .map(|(id, owner)| format!("{{\"id\":\"{id}\",\"owned_by\":\"{owner}\"}}"))
        .collect();
    format!("{{\"data\":[{}]}}", entries.join(","))
}

pub(crate) fn server_with_models(models: &[(&str, &str)]) -> TestServer {
    TestServer::start(vec![Answer::json(200, &models_body(models))])
}

pub(crate) fn client(server: &TestServer) -> Client {
    client_for(server, Some("sk-test"))
}
