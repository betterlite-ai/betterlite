use std::collections::BTreeMap;

use redis::Value;

use super::error::ValkeyError;
use super::ports::ValkeyDocument;

pub(super) fn documents(reply: &Value) -> Result<Vec<ValkeyDocument>, ValkeyError> {
    if let Value::Map(entries) = reply {
        return documents_from_map(entries);
    }
    if let Some(items) = reply.as_sequence() {
        return documents_from_array(items);
    }
    Err(ValkeyError::UnexpectedReply {
        detail: "the reply is neither an array nor a map",
    })
}

fn documents_from_array(items: &[Value]) -> Result<Vec<ValkeyDocument>, ValkeyError> {
    let mut documents = Vec::new();
    let mut rest = items.iter().skip(1);
    loop {
        let Some(id) = rest.next() else {
            return Ok(documents);
        };
        let Some(fields) = rest.next() else {
            return Err(ValkeyError::UnexpectedReply {
                detail: "a document id has no fields",
            });
        };
        documents.push(document(id, fields)?);
    }
}

fn documents_from_map(entries: &[(Value, Value)]) -> Result<Vec<ValkeyDocument>, ValkeyError> {
    let mut documents = Vec::new();
    for (name, value) in entries {
        if text(name)? != "results" {
            continue;
        }
        let Some(results) = value.as_sequence() else {
            return Err(ValkeyError::UnexpectedReply {
                detail: "the results member is not an array",
            });
        };
        for result in results {
            documents.push(document_from_map(result)?);
        }
    }
    Ok(documents)
}

fn document_from_map(result: &Value) -> Result<ValkeyDocument, ValkeyError> {
    let Some(entries) = result.as_map_iter() else {
        return Err(ValkeyError::UnexpectedReply {
            detail: "a search result is not a map",
        });
    };
    let mut id = None;
    let mut fields = BTreeMap::new();
    for (name, value) in entries {
        match text(name)?.as_str() {
            "id" => id = Some(text(value)?),
            "extra_attributes" => fields = attributes(value)?,
            _ => {}
        }
    }
    let document_id = id.ok_or_else(|| ValkeyError::UnexpectedReply {
        detail: "a search result has no id",
    })?;
    Ok(ValkeyDocument {
        id: document_id,
        fields,
    })
}

fn attributes(value: &Value) -> Result<BTreeMap<String, String>, ValkeyError> {
    let Some(entries) = value.as_map_iter() else {
        return Err(ValkeyError::UnexpectedReply {
            detail: "a search result's attributes are not a map",
        });
    };
    let mut fields = BTreeMap::new();
    for (name, field) in entries {
        fields.insert(text(name)?, text(field)?);
    }
    Ok(fields)
}

fn document(id: &Value, values: &Value) -> Result<ValkeyDocument, ValkeyError> {
    let Some(items) = values.as_sequence() else {
        return Err(ValkeyError::UnexpectedReply {
            detail: "a document's fields are not an array",
        });
    };
    let mut pairs = items.iter();
    let mut fields = BTreeMap::new();
    while let Some(name) = pairs.next() {
        let Some(field) = pairs.next() else {
            return Err(ValkeyError::UnexpectedReply {
                detail: "a field name has no value",
            });
        };
        fields.insert(text(name)?, text(field)?);
    }
    Ok(ValkeyDocument {
        id: text(id)?,
        fields,
    })
}

fn text(value: &Value) -> Result<String, ValkeyError> {
    if let Value::BulkString(bytes) = value {
        // redis-py decodes field bytes with errors="ignore", so the invalid bytes are dropped
        let mut decoded = String::with_capacity(bytes.len());
        for chunk in bytes.utf8_chunks() {
            decoded.push_str(chunk.valid());
        }
        return Ok(decoded);
    }
    if let Value::SimpleString(text) = value {
        return Ok(text.clone());
    }
    Err(ValkeyError::UnexpectedReply {
        detail: "a field value is not text",
    })
}
