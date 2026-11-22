import litellm
from pydantic import JsonValue

from azure_batches import run as batches_run
from azure_files import run as files_run

from .builders import use_batches_builder, use_files_builder
from .contracts import (
    BATCH_ACTIONS,
    BATCH_CREATE,
    BATCHES_IN,
    FIELDS,
    FILE_ACTIONS,
    FILES_IN,
    ROWS,
    STRING,
    VALUE,
    first_step,
    page_value,
    paged,
    raise_error,
    reply_payload,
    returned_of,
)

litellm.suppress_debug_info = True


def file_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    result = FIELDS.validate_python(row["result"])
    if kind == "file_content":
        body = STRING.validate_python(result["body"])
        return {"status": result["status"], "length": len(body)}
    if kind == "file_list":
        return {"count": len(ROWS.validate_python(result["data"]))}
    if kind == "file_delete":
        return {"deleted": result["deleted"]}
    return {"id": result["id"], "bytes": result["bytes"]}


def file_step(kind: str, step: dict[str, JsonValue]) -> dict[str, JsonValue]:
    body = STRING.validate_python(step["body"])
    reply: dict[str, JsonValue] = {
        "status": step["status"],
        "headers": {"content-type": "application/json"},
        "body": body,
    }
    payload = reply_payload(FILE_ACTIONS[kind], step, reply)
    if kind == "file_create":
        payload["content"] = "plain"
        payload["create_purpose"] = "batch"
    elif kind != "file_list":
        payload["file_id"] = STRING.validate_python(step["target_id"])
    return payload


def file_answer(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    use_files_builder()
    argument: dict[str, JsonValue] = {"steps": [file_step(kind, step)]}
    answer = files_run(FILES_IN.validate_python(argument))
    row = first_step(answer, "steps")
    raise_error(row)
    return file_value(kind, row)


def batch_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    batch = returned_of(row)
    if kind == "batch_list":
        return page_value(batch)
    return {"id": batch["id"], "status": batch["status"]}


def batch_step(kind: str, step: dict[str, JsonValue]) -> dict[str, JsonValue]:
    body = VALUE.validate_json(STRING.validate_python(step["body"]))
    reply: dict[str, JsonValue] = {"status": step["status"], "body": body}
    action = BATCH_ACTIONS[kind]
    payload = reply_payload(action, step, reply)
    if action == "create":
        payload["data"] = VALUE.validate_json(BATCH_CREATE)
    elif action == "list":
        paged(payload, step)
    else:
        payload["query"] = {"batch_id": STRING.validate_python(step["target_id"])}
    return payload


def batch_answer(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    use_batches_builder()
    argument: dict[str, JsonValue] = {"steps": [batch_step(kind, step)]}
    answer = batches_run(BATCHES_IN.validate_python(argument))
    return batch_value(kind, first_step(answer, "steps"))


def one_step(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    if kind in FILE_ACTIONS:
        return file_answer(kind, step)
    return batch_answer(kind, step)
