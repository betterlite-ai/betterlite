use redis::{Cmd, cmd};

use crate::cache_primitives::protocol::RedisScalar;

pub(crate) fn rpush_command(key: &str, values: &[RedisScalar]) -> Cmd {
    let mut command = cmd("RPUSH");
    command.arg(key);
    for value in values {
        match value {
            RedisScalar::Text(text) => command.arg(text),
            RedisScalar::Bytes(bytes) => command.arg(bytes),
            RedisScalar::Float(number) => command.arg(*number),
        };
    }
    command
}

pub(crate) fn ltrim_command(key: &str, start: i64, stop: i64) -> Cmd {
    let mut command = cmd("LTRIM");
    command.arg(key).arg(start).arg(stop);
    command
}

pub(crate) fn lpop_command(key: &str, count: Option<i64>) -> Cmd {
    let mut command = cmd("LPOP");
    command.arg(key);
    if let Some(limit) = count {
        command.arg(limit);
    }
    command
}
