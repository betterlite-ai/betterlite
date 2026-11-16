use std::io::{self, BufRead as _, IsTerminal as _, Write as _};

use console::Term;

use crate::cli::root::root_command;
use crate::cli::session::Session;
use crate::cli::{Failure, dispatch};
use crate::commands_listing::show_commands;

const BANNER: &str = "   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2557}     \u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\n   \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{2550}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{2550}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{255d}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}\u{255a}\u{2550}\u{2550}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{255d}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\n   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2554}\u{255d}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}     \u{2588}\u{2588}\u{2551}      \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2554}\u{255d}\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\n   \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{255d}     \u{2588}\u{2588}\u{2551}      \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{255d}  \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{255d}\n   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2554}\u{255d}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}   \u{2588}\u{2588}\u{2551}      \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2551}  \u{2588}\u{2588}\u{2551}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\n   \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d} \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}   \u{255a}\u{2550}\u{255d}      \u{255a}\u{2550}\u{255d}   \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{255d}  \u{255a}\u{2550}\u{255d}\u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{255d}   \u{255a}\u{2550}\u{255d}   \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}";
const WIDTH: usize = 80;
const BORDER_WIDTH: usize = WIDTH - 2;
const INNER_WIDTH: usize = WIDTH - 4;
const MIN_HEIGHT: u16 = 10;

pub trait LineReader {
    fn read_line(&mut self) -> io::Result<Option<String>>;
}

pub struct StdinReader;

impl LineReader for StdinReader {
    #[cfg_attr(test, mutants::skip)]
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let read = io::stdin().lock().read_line(&mut line)?;
        if read == 0 {
            return Ok(None);
        }
        Ok(Some(line))
    }
}

pub(crate) fn run(session: &mut Session<'_>, reader: &mut dyn LineReader) -> Result<(), Failure> {
    setup(session)?;
    loop {
        writeln!(session.streams.stdout, "\n")?;
        let Some(input) = prompt(session, reader)? else {
            writeln!(session.streams.stdout, "\nGoodbye!")?;
            return Ok(());
        };
        let line = input.trim();
        if line.is_empty() {
            continue;
        }
        match special(session, line)? {
            Some(true) => return Ok(()),
            Some(false) => {}
            None => execute(session, line)?,
        }
    }
}

fn setup(session: &mut Session<'_>) -> Result<(), Failure> {
    writeln!(session.streams.stdout, "\n{BANNER}\n")?;
    writeln!(
        session.streams.stdout,
        "Connected to betterlite server: {}\n",
        session.context.base_url
    )?;
    show_commands(&mut session.streams.stdout, session.environment)?;
    Ok(())
}

fn clear_screen(out: &mut dyn io::Write, terminal: bool) -> io::Result<()> {
    if terminal {
        write!(out, "\x1b[2J\x1b[1;1H")?;
    }
    Ok(())
}

fn special(session: &mut Session<'_>, line: &str) -> Result<Option<bool>, Failure> {
    match line.to_lowercase().as_str() {
        "exit" | "quit" => {
            writeln!(session.streams.stdout, "Goodbye!")?;
            Ok(Some(true))
        }
        "help" => {
            writeln!(session.streams.stdout)?;
            show_commands(&mut session.streams.stdout, session.environment)?;
            Ok(Some(false))
        }
        "clear" => {
            clear_screen(&mut session.streams.stdout, io::stdout().is_terminal())?;
            writeln!(session.streams.stdout, "\n{BANNER}\n")?;
            show_commands(&mut session.streams.stdout, session.environment)?;
            Ok(Some(false))
        }
        _ => Ok(None),
    }
}

fn execute(session: &mut Session<'_>, line: &str) -> Result<(), Failure> {
    let mut argv = vec![String::from("lite")];
    argv.extend(line.split_whitespace().map(str::to_owned));
    let root = root_command(session.environment);
    let Some(name) = argv.get(1) else {
        return Ok(());
    };
    if !root
        .get_subcommands()
        .any(|subcommand| subcommand.get_name() == name)
    {
        writeln!(session.streams.stdout, "Unknown command: {name}")?;
        writeln!(
            session.streams.stdout,
            "Type 'help' to see available commands."
        )?;
        return Ok(());
    }
    let matches = match root.try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(error) => return print_clap(session, &error),
    };
    match dispatch::run(session, &matches) {
        Ok(_code) => Ok(()),
        Err(failure) => report_in_session(session, &failure),
    }
}

fn print_clap(session: &mut Session<'_>, error: &clap::Error) -> Result<(), Failure> {
    let rendered = error.render().to_string();
    if error.use_stderr() {
        write!(session.streams.stderr, "{rendered}")?;
    } else {
        write!(session.streams.stdout, "{rendered}")?;
    }
    Ok(())
}

fn report_in_session(session: &mut Session<'_>, failure: &Failure) -> Result<(), Failure> {
    match failure {
        Failure::Aborted(_) => writeln!(session.streams.stdout, "Command aborted.")?,
        Failure::Plain(message) | Failure::Usage(message) => {
            writeln!(session.streams.stderr, "Error: {message}")?;
        }
    }
    Ok(())
}

const fn too_short(size: Option<(u16, u16)>) -> bool {
    let Some((rows, _columns)) = size else {
        return true;
    };
    rows < MIN_HEIGHT
}

fn prompt(session: &mut Session<'_>, reader: &mut dyn LineReader) -> io::Result<Option<String>> {
    let out = &mut session.streams.stdout;
    if too_short(Term::stdout().size_checked()) {
        writeln!(out, "\n\n\n")?;
    }
    let rule = format!("+{}+", "-".repeat(BORDER_WIDTH));
    writeln!(out, "{rule}")?;
    writeln!(out, "| {} |", " ".repeat(INNER_WIDTH))?;
    writeln!(out, "{rule}")?;
    write!(out, "\x1b[2A\r| > ")?;
    let line = reader.read_line()?;
    writeln!(out, "\x1b[1B\n")?;
    Ok(line)
}

mod tests;
