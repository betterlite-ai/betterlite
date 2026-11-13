use crate::commands::Client;

use super::conversation::{ConversationError, ConversationStore};
use super::history;
use super::models;
use super::stream::{self, StreamRequest};
use super::{ChatArgs, ChatError, Message, Terminal};

const COMMAND_PREFIXES: [&str; 9] = [
    "/quit", "/exit", "/q", "/help", "/clear", "/history", "/save", "/load", "/model",
];
const SAVE_USAGE: &str = "Usage: /save <filename>";
const LOAD_USAGE: &str = "Usage: /load <filename>";

#[derive(Clone, Copy, Eq, PartialEq)]
enum Flow {
    Continue,
    Exit,
}

struct Session<'a> {
    client: &'a Client,
    terminal: &'a mut dyn Terminal,
    store: &'a mut dyn ConversationStore,
    system: Option<String>,
    model: String,
    messages: Vec<Message>,
    temperature: f64,
    max_tokens: Option<i64>,
}

pub(super) fn run(
    client: &Client,
    terminal: &mut dyn Terminal,
    store: &mut dyn ConversationStore,
    args: &ChatArgs,
    model: String,
) -> Result<(), ChatError> {
    let mut session = Session {
        client,
        terminal,
        store,
        system: args.system.clone(),
        model,
        messages: Vec::new(),
        temperature: args.temperature,
        max_tokens: args.max_tokens,
    };
    session.start()
}

impl Session<'_> {
    fn start(&mut self) -> Result<(), ChatError> {
        history::write_welcome(
            self.terminal,
            &self.model,
            self.temperature,
            self.max_tokens,
        )?;
        loop {
            if self.turn()? == Flow::Exit {
                return Ok(());
            }
        }
    }

    fn turn(&mut self) -> Result<Flow, ChatError> {
        self.terminal.write_fragment("\nYou: ")?;
        let Some(line) = self.terminal.read_line()? else {
            self.terminal.write_line("Chat session ended.")?;
            return Ok(Flow::Exit);
        };
        let input = line.trim();
        let lower = input.to_lowercase();
        if self.handle(input, &lower)? == Flow::Exit {
            return Ok(Flow::Exit);
        }
        if input.is_empty()
            || COMMAND_PREFIXES
                .iter()
                .any(|prefix| lower.starts_with(prefix))
        {
            return Ok(Flow::Continue);
        }
        self.messages.push(Message {
            role: String::from("user"),
            content: input.to_owned(),
        });
        self.reply()?;
        Ok(Flow::Continue)
    }

    fn reply(&mut self) -> Result<(), ChatError> {
        self.terminal.write_line("Assistant:")?;
        let content = stream::stream_reply(
            self.client,
            self.terminal,
            StreamRequest {
                model: &self.model,
                messages: &self.messages,
                temperature: self.temperature,
                max_tokens: self.max_tokens,
            },
        )?;
        self.terminal.write_line("")?;
        if content.is_empty() {
            self.terminal
                .write_line("Error: No content received from the model")?;
            return Ok(());
        }
        self.messages.push(Message {
            role: String::from("assistant"),
            content,
        });
        Ok(())
    }

    fn handle(&mut self, input: &str, lower: &str) -> Result<Flow, ChatError> {
        if lower == "/quit" || lower == "/exit" || lower == "/q" {
            self.terminal.write_line("Chat session ended.")?;
            return Ok(Flow::Exit);
        }
        if lower == "/help" {
            history::write_help(self.terminal)?;
        } else if lower == "/clear" {
            self.clear()?;
        } else if lower == "/history" {
            history::write_history(self.terminal, &self.messages)?;
        } else if lower.starts_with("/save") {
            self.save(input)?;
        } else if lower.starts_with("/load") {
            self.load(input)?;
        } else if lower == "/model" {
            self.switch_model()?;
        }
        Ok(Flow::Continue)
    }

    fn clear(&mut self) -> Result<(), ChatError> {
        self.fallback();
        self.terminal.write_line("Conversation history cleared.")
    }

    fn fallback(&mut self) {
        self.messages = match &self.system {
            Some(system) => vec![Message {
                role: String::from("system"),
                content: system.clone(),
            }],
            None => Vec::new(),
        };
    }

    fn save(&mut self, input: &str) -> Result<(), ChatError> {
        let Some(filename) = self.filename_or_usage(input, SAVE_USAGE)? else {
            return Ok(());
        };
        if let Err(error) = self.store.save(&filename, &self.messages) {
            return self
                .terminal
                .write_line(&format!("Error saving conversation: {error}"));
        }
        self.terminal
            .write_line(&format!("Conversation saved to {filename}"))
    }

    fn load(&mut self, input: &str) -> Result<(), ChatError> {
        let Some(filename) = self.filename_or_usage(input, LOAD_USAGE)? else {
            return Ok(());
        };
        match self.store.load(&filename) {
            Ok(messages) => {
                self.messages = messages;
                self.terminal
                    .write_line(&format!("Conversation loaded from {filename}"))
            }
            Err(error) => self.report_load(&error, &filename),
        }
    }

    fn report_load(&mut self, error: &ConversationError, filename: &str) -> Result<(), ChatError> {
        let text = match error {
            ConversationError::NotFound => format!("File not found: {filename}"),
            other => format!("Error loading conversation: {other}"),
        };
        self.terminal.write_line(&text)?;
        self.fallback();
        Ok(())
    }

    fn filename_or_usage(&mut self, input: &str, usage: &str) -> Result<Option<String>, ChatError> {
        let Some(filename) = conversation_filename(input) else {
            self.terminal.write_line(usage)?;
            return Ok(None);
        };
        Ok(Some(filename))
    }

    fn switch_model(&mut self) -> Result<(), ChatError> {
        let Some(model) = models::pick(self.client, self.terminal)? else {
            return Ok(());
        };
        self.model = model;
        self.terminal
            .write_line(&format!("Switched to model: {}", self.model))
    }
}

fn conversation_filename(input: &str) -> Option<String> {
    let mut parts = input.split_whitespace();
    let _command = parts.next();
    let name = parts.next()?;
    if name.ends_with(".json") {
        return Some(name.to_owned());
    }
    Some(format!("{name}.json"))
}
