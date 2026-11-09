use super::Lists;
use super::command::Command;
use super::replies::error_reply;

pub(super) fn push_reply(lists: &mut Lists, key: &str, command: &Command) -> Vec<u8> {
    let values: Vec<Vec<u8>> = command.args.iter().skip(2).cloned().collect();
    if values.is_empty() {
        return error_reply("ERR wrong number of arguments");
    }
    let rows = lists.entry(key.to_owned()).or_default();
    rows.extend(values);
    i64::try_from(rows.len()).map_or_else(
        |_error| error_reply("ERR the list is too long"),
        |value| format!(":{value}\r\n").into_bytes(),
    )
}
