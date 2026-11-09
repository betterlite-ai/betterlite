use std::str;

use super::Lists;
use super::command::Command;
use super::pop::pop_reply;
use super::push::push_reply;
use super::replies::error_reply;
use super::trim::trim_reply;

pub(super) fn apply(lists: &mut Lists, command: &Command) -> Vec<u8> {
    let key = command
        .args
        .get(1)
        .and_then(|argument| str::from_utf8(argument).ok());
    match command.name.as_str() {
        "RPUSH" => key.map_or_else(
            || error_reply("ERR wrong number of arguments"),
            |name| push_reply(lists, name, command),
        ),
        "LTRIM" => key.map_or_else(
            || error_reply("ERR wrong number of arguments"),
            |name| trim_reply(lists, name, command),
        ),
        "LPOP" => key.map_or_else(
            || error_reply("ERR wrong number of arguments"),
            |name| pop_reply(lists, name, command),
        ),
        _ => b"+OK\r\n".to_vec(),
    }
}
