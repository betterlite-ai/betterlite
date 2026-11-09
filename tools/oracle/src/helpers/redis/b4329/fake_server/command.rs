use std::io;
use std::str;

use tokio::io::{AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _};

#[derive(Clone)]
pub(super) struct Command {
    pub(super) name: String,
    pub(super) args: Vec<Vec<u8>>,
}

impl Command {
    pub(super) fn number(&self, index: usize) -> Option<i64> {
        let raw = self.args.get(index)?;
        let text = str::from_utf8(raw).ok()?;
        text.parse().ok()
    }
}

pub(super) async fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: AsyncBufRead + Unpin,
{
    let mut header = String::new();
    if reader.read_line(&mut header).await? == 0 {
        return Ok(None);
    }
    let Some(count_text) = header.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let mut length_line = String::new();
        if reader.read_line(&mut length_line).await? == 0 {
            return Ok(None);
        }
        let Some(length_text) = length_line.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await?;
        buffer.truncate(length);
        args.push(buffer);
    }
    let Some(raw_name) = args.first() else {
        return Ok(None);
    };
    let command_name = String::from_utf8(raw_name.clone())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(Command {
        name: command_name.to_ascii_uppercase(),
        args,
    }))
}
