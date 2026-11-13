use redis::Value;

#[derive(Debug)]
pub(crate) enum RedisReply {
    Nil,
    Int(i64),
    Bulk(Vec<u8>),
    Array(Vec<Value>),
    Double(f64),
    Unsupported,
}

pub(crate) fn classify(value: Value) -> RedisReply {
    match value {
        Value::Nil => RedisReply::Nil,
        Value::Int(number) => RedisReply::Int(number),
        Value::BulkString(bytes) => RedisReply::Bulk(bytes),
        Value::Array(values) => RedisReply::Array(values),
        Value::Double(number) => RedisReply::Double(number),
        Value::SimpleString(_)
        | Value::Okay
        | Value::Map(_)
        | Value::Attribute { .. }
        | Value::Set(_)
        | Value::Boolean(_)
        | Value::VerbatimString { .. }
        | Value::BigNumber(_)
        | Value::Push { .. }
        | Value::ServerError(_)
        | _ => RedisReply::Unsupported,
    }
}
