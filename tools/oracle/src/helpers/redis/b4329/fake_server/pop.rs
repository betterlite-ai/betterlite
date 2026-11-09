use super::Lists;
use super::command::Command;
use super::replies::{array_reply, bulk_reply, error_reply};

enum Popped {
    Missing,
    One(Vec<u8>),
    Many(Vec<Vec<u8>>),
}

pub(super) fn pop_reply(lists: &mut Lists, key: &str, command: &Command) -> Vec<u8> {
    let count = if command.args.len() > 2 {
        match command.number(2) {
            Some(value) => Some(value),
            None => return error_reply("ERR value is not an integer or out of range"),
        }
    } else {
        None
    };
    if matches!(count, Some(value) if value < 0) {
        return error_reply("ERR value is out of range, must be positive");
    }
    match pop_rows(lists, key, count) {
        Popped::Missing => b"$-1\r\n".to_vec(),
        Popped::One(bytes) => bulk_reply(&bytes),
        Popped::Many(values) => array_reply(&values),
    }
}

fn pop_rows(lists: &mut Lists, key: &str, count: Option<i64>) -> Popped {
    let Some(rows) = lists.get_mut(key) else {
        return Popped::Missing;
    };
    if rows.is_empty() {
        return Popped::Missing;
    }
    match count {
        None => Popped::One(rows.remove(0)),
        Some(limit) => {
            let take = usize::try_from(limit).map_or(rows.len(), |value| value.min(rows.len()));
            Popped::Many(rows.drain(..take).collect())
        }
    }
}
