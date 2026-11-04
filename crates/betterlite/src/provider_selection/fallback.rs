use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};

use crate::terminal_styling::{bold, grey};

use super::PROVIDERS;

#[derive(Debug, thiserror::Error)]
pub enum FallbackError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    Empty,
    Invalid,
    OutOfRange,
    Valid(BTreeSet<usize>),
}

#[must_use]
pub fn parse_selection(input: &str) -> ParseOutcome {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return ParseOutcome::Empty;
    }
    let mut numbers = Vec::new();
    for part in trimmed.replace(' ', ",").split(',') {
        if part.is_empty() {
            continue;
        }
        let Ok(value) = part.parse::<usize>() else {
            return ParseOutcome::Invalid;
        };
        numbers.push(value);
    }
    let valid: BTreeSet<usize> = numbers
        .into_iter()
        .filter(|value| *value >= 1 && *value <= PROVIDERS.len())
        .filter_map(|value| value.checked_sub(1))
        .collect();
    if valid.is_empty() {
        ParseOutcome::OutOfRange
    } else {
        ParseOutcome::Valid(valid)
    }
}

fn print_provider_list(writer: &mut dyn Write, supports_color: bool) -> io::Result<()> {
    writeln!(writer)?;
    writeln!(writer, "  {}", bold("Add your first model", supports_color))?;
    writeln!(
        writer,
        "{}",
        grey(
            "  Enter numbers separated by commas (e.g. 1,2). Press Enter to confirm.",
            supports_color
        )
    )?;
    writeln!(writer)?;
    for (index, provider) in PROVIDERS.iter().enumerate() {
        writeln!(
            writer,
            "  {} {}  {}",
            grey(&format!("{}.", index.saturating_add(1)), supports_color),
            bold(provider.name, supports_color),
            grey(provider.description, supports_color)
        )?;
    }
    writeln!(writer)?;
    Ok(())
}

fn print_retry_message(
    writer: &mut dyn Write,
    supports_color: bool,
    outcome: &ParseOutcome,
) -> io::Result<()> {
    match outcome {
        ParseOutcome::Empty => writeln!(
            writer,
            "{}",
            grey("  Please select at least one provider.", supports_color)
        ),
        ParseOutcome::OutOfRange => writeln!(
            writer,
            "{}",
            grey(
                &format!("  Enter numbers between 1 and {}.", PROVIDERS.len()),
                supports_color
            )
        ),
        ParseOutcome::Invalid => writeln!(
            writer,
            "{}",
            grey(
                "  Enter numbers separated by commas, e.g. 1,3",
                supports_color
            )
        ),
        ParseOutcome::Valid(_) => Ok(()),
    }
}

pub fn select_fallback(
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    supports_color: bool,
) -> Result<BTreeSet<usize>, FallbackError> {
    print_provider_list(writer, supports_color)?;

    loop {
        write!(
            writer,
            "  {} Provider(s): ",
            bold("\u{276f}", supports_color)
        )?;
        writer.flush()?;
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(FallbackError::Io(io::Error::from(
                io::ErrorKind::UnexpectedEof,
            )));
        }
        let outcome = parse_selection(&line);
        if let ParseOutcome::Valid(indices) = outcome {
            return Ok(indices);
        }
        print_retry_message(writer, supports_color, &outcome)?;
    }
}

mod tests;
