use super::Lists;
use super::command::Command;
use super::replies::error_reply;

pub(super) fn trim_reply(lists: &mut Lists, key: &str, command: &Command) -> Vec<u8> {
    let (Some(start), Some(stop)) = (command.number(2), command.number(3)) else {
        return error_reply("ERR wrong number of arguments");
    };
    if let Some(rows) = lists.get_mut(key) {
        trim_rows(rows, start, stop);
    }
    b"+OK\r\n".to_vec()
}

fn trim_rows(rows: &mut Vec<Vec<u8>>, start: i64, stop: i64) {
    let Ok(length) = i64::try_from(rows.len()) else {
        rows.clear();
        return;
    };
    let begin = if start < 0 {
        length.saturating_add(start)
    } else {
        start
    }
    .max(0);
    let end = if stop < 0 {
        length.saturating_add(stop)
    } else {
        stop
    }
    .min(length.saturating_sub(1));
    if begin > end {
        rows.clear();
        return;
    }
    let (Ok(begin_index), Ok(end_index)) = (usize::try_from(begin), usize::try_from(end)) else {
        rows.clear();
        return;
    };
    rows.truncate(end_index.saturating_add(1));
    rows.drain(..begin_index);
}
