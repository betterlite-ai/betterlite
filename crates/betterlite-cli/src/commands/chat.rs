use std::io::{self, BufRead as _, Write as _};

use betterlite_http::error::HttpError;
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::cli_context::CliContext;

use super::Client;

pub mod conversation;
pub mod stream_line;

mod history;
mod models;
mod session;
mod stream;

mod tests;

#[derive(Debug, Args)]
#[command(about = "Interactive chat with streaming responses")]
pub struct ChatArgs {
    #[arg(help = "Model to use; the model picker runs when it is omitted")]
    model: Option<String>,
    #[arg(
        short,
        long,
        default_value_t = 0.7,
        help = "Sampling temperature between 0 and 2 (default: 0.7)"
    )]
    temperature: f64,
    #[arg(long, help = "Maximum number of tokens to generate")]
    max_tokens: Option<i64>,
    #[arg(
        short,
        long,
        help = "System message to set the behavior of the assistant"
    )]
    system: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    role: String,
    content: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error(transparent)]
    Client(#[from] super::ClientError),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("the proxy answered HTTP {status}")]
    Status { status: u16 },
    #[error("the models response has no data field")]
    ModelsData,
    #[error("the models response could not be read")]
    Models(#[source] serde_json::Error),
    #[error("the chat request could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the reply did not arrive within {seconds} seconds")]
    Timeout { seconds: u64 },
    #[error("the response headers are larger than {limit} bytes")]
    Headers { limit: usize },
    #[error("the error body is larger than {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error("the streamed reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error(transparent)]
    Line(#[from] stream_line::StreamLineError),
    #[error("the error body is not valid UTF-8")]
    NotUtf8,
    #[error("the terminal could not be used")]
    Terminal(#[from] io::Error),
}

pub trait Terminal {
    fn read_line(&mut self) -> io::Result<Option<String>>;
    fn write_line(&mut self, text: &str) -> io::Result<()>;
    fn write_fragment(&mut self, text: &str) -> io::Result<()>;
    fn write_error_line(&mut self, text: &str) -> io::Result<()>;
}

pub struct StdTerminal;

impl Terminal for StdTerminal {
    #[cfg_attr(test, mutants::skip)]
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let read = io::stdin().lock().read_line(&mut line)?;
        if read == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim_end_matches(['\r', '\n']).to_owned()))
    }

    #[cfg_attr(test, mutants::skip)]
    fn write_line(&mut self, text: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(text.as_bytes())?;
        stdout.write_all(b"\n")?;
        stdout.flush()
    }

    #[cfg_attr(test, mutants::skip)]
    fn write_fragment(&mut self, text: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(text.as_bytes())?;
        stdout.flush()
    }

    #[cfg_attr(test, mutants::skip)]
    fn write_error_line(&mut self, text: &str) -> io::Result<()> {
        let mut stderr = io::stderr().lock();
        stderr.write_all(text.as_bytes())?;
        stderr.write_all(b"\n")?;
        stderr.flush()
    }
}

pub fn run_chat_command(
    args: &ChatArgs,
    _cli_context: &CliContext,
    client: &Client,
    terminal: &mut dyn Terminal,
    store: &mut dyn conversation::ConversationStore,
) -> Result<(), ChatError> {
    let model = match args.model.clone() {
        Some(model) => model,
        None => match models::pick(client, terminal)? {
            Some(model) if !model.is_empty() => model,
            _ => {
                terminal.write_line("No model selected. Exiting.")?;
                return Ok(());
            }
        },
    };
    session::run(client, terminal, store, args, model)
}
