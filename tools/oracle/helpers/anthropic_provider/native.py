from typing import Final, TypedDict

from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue, TypeAdapter

import anthropic_batches
import anthropic_count_tokens
import anthropic_files

from .types import Input, Observation, Recorded

JSON: Final = TypeAdapter(JsonValue)


class CountRequest(TypedDict):
    url: str
    headers: dict[str, str | None]
    body: JsonValue


class CountReply(TypedDict):
    value: dict[str, JsonValue]


class CountCall(TypedDict):
    request: CountRequest
    response: CountReply


class CountOutput(TypedDict):
    calls: list[CountCall]


class BatchRequest(TypedDict):
    method: str
    url: str
    headers: dict[str, str]
    auth_header: str


class BatchCounts(TypedDict):
    completed: int
    failed: int
    total: int


class Batch(TypedDict):
    id: str
    status: str
    output_file_id: str
    request_counts: BatchCounts


class BatchScenario(TypedDict):
    requests: list[BatchRequest]
    batch: LiteLLMBatch


class BatchOutput(TypedDict):
    scenarios: list[BatchScenario]




class File(TypedDict):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: str
    status: str


class FileReplies(TypedDict):
    retrieve: list[File]


class FileList(TypedDict):
    url: str
    queries: list[list[list[str]]]
    files: list[File]


class Route(TypedDict):
    url: str
    query: list[list[str]]


class Deleted(TypedDict):
    id: str
    object: str
    deleted: bool


class FileDelete(TypedDict):
    retrieve: list[Route]
    delete: list[Route]
    responses: list[Deleted]


class ContentBody(TypedDict):
    body: list[int]


class Content(TypedDict):
    urls: list[Route]
    result: ContentBody


class Part(TypedDict):
    name: str
    filename: str | None
    content_type: str | None
    content: list[int]


class Upload(TypedDict):
    parts: list[Part]


class Uploads(TypedDict):
    uploads: list[Upload]


class Headers(TypedDict):
    headers: dict[str, str]
    urls: list[str]


class FileHeaders(TypedDict):
    scenarios: list[Headers]


def file_headers(supplied: Input) -> Headers:
    auth = TypeAdapter(FileHeaders).validate_python(anthropic_files.run({"file_beta_header": {
        "scenarios": [{"api_key": supplied["api_key"], "expected_auth": supplied["expected_key"],
            "caller_headers": [[name, value] for name, value in supplied["headers"].items()],
            "url_bases": [supplied["api_base"]],
            "error": {"status": 400, "message": "fixture", "headers": []},
        }],
    }}))["scenarios"][0]
    headers = auth["headers"]
    headers["x-api-key"] = (
        "matched" if headers["x-api-key"] == supplied["expected_key"] else "mismatch"
    )
    return auth


def observe(supplied: Input) -> Observation:
    observed: Recorded | None
    result: JsonValue
    if supplied["action"] == "count":
        payload: dict[str, JsonValue] = {
            "function": "handle", "cases": [{
                "model": supplied["model"], "messages": JSON.validate_python(supplied["messages"]),
                "api_key": supplied["api_key"], "api_base": supplied["api_base"],
                "system": supplied["system"], "tools": JSON.validate_python(supplied["tools"]),
                "optional_params": supplied["optional_params"],
                "reply": {"body": supplied["response"], "status": 200},
            }],
        }
        count_answer = TypeAdapter(CountOutput).validate_python(
            anthropic_count_tokens.run(payload),
        )
        count_call = count_answer["calls"][0]
        count_request = count_call["request"]
        headers = {
            name: value for name, value in count_request["headers"].items() if value is not None
        }
        key = headers["x-api-key"]
        headers["x-api-key"] = "matched" if key == supplied["expected_key"] else "mismatch"
        observed = {
            "method": "POST", "url": count_request["url"], "query": [],
            "headers": headers, "body": count_request["body"],
        }
        result = count_call["response"]["value"]
    elif supplied["action"] == "batch_create":
        anthropic_batches.run({"create_requests": {"environment": {}, "url_bases": []}})
        result = {"error": True, "status": None}
        observed = None
    elif supplied["action"] == "batch_retrieve":
        batch_payload: dict[str, JsonValue] = {"retrieve": {
            "now": 1_700_000_000, "scenarios": [{
                "batch_id": supplied["id"], "api_base": supplied["api_base"],
                "api_key": supplied["api_key"], "environment": {},
                "body": supplied["response"], "sync": False,
            }],
        }}
        batch_answer = TypeAdapter(BatchOutput).validate_python(
            anthropic_batches.run(batch_payload),
        )
        batch_scenario = batch_answer["scenarios"][0]
        batch_value = TypeAdapter(Batch).validate_json(batch_scenario["batch"].model_dump_json())
        batch_request = batch_scenario["requests"][0]
        headers = batch_request["headers"]
        headers[batch_request["auth_header"]] = "matched"
        observed = {
            "method": batch_request["method"], "url": batch_request["url"],
            "query": [], "headers": headers, "body": None,
        }
        result = JSON.validate_python(batch_value)
    else:
        auth = file_headers(supplied)
        headers = auth["headers"]
        observed = {
            "method": "GET", "url": auth["urls"][0], "query": [], "headers": headers, "body": None,
        }
        if supplied["action"] == "list":
            reply = TypeAdapter(FileList).validate_python(anthropic_files.run({
                "file_list_pagination": {
                "now": 1_700_000_000, "purposes": [supplied["purpose"]],
                "params_api_base": supplied["api_base"],
                "body": JSON.validate_json(supplied["response"]),
            }}))
            observed["url"], observed["query"] = reply["url"], reply["queries"][0]
            result = JSON.validate_python(reply["files"])
        elif supplied["action"] in {"content", "batch_results"}:
            content = TypeAdapter(Content).validate_python(anthropic_files.run({
                "file_content_bytes": {
                "file_ids": [supplied["id"]], "params_api_base": supplied["api_base"],
                "status": 200, "body": list(supplied["response"].encode()), "headers": [],
                "error": {"status": 400, "message": "fixture", "headers": []},
            }}))
            observed["url"] = content["urls"][0]["url"]
            result = bytes(content["result"]["body"]).decode()
        else:
            body = JSON.validate_json(supplied["response"])
            route = TypeAdapter(FileDelete).validate_python(anthropic_files.run({
                "file_delete_shape": {
                "file_ids": [supplied["id"]], "params_api_base": supplied["api_base"], "bodies": [body],
            }}))
            if supplied["action"] == "delete":
                observed["method"], observed["url"] = "DELETE", route["delete"][0]["url"]
                result = JSON.validate_python(route["responses"][0])
            else:
                replies = TypeAdapter(FileReplies).validate_python(anthropic_files.run({
                    "native_file_object": {"now": 1_700_000_000, "bodies": [body]},
                }))
                result = JSON.validate_python(replies["retrieve"][0])
                if supplied["action"] == "retrieve":
                    observed["url"] = route["retrieve"][0]["url"]
                else:
                    upload = TypeAdapter(Uploads).validate_python(anthropic_files.run({
                        "file_upload_multipart": {
                        "now": 1_700_000_000, "uploads": [{
                            "data": supplied["file"], "filename": "input.jsonl",
                            "content_type": "application/jsonl", "purpose": supplied["purpose"],
                        }],
                    }}))
                    observed["method"] = "POST"
                    observed["body"] = {"boundary": "oracle-boundary", "parts": [{
                        "name": part["name"], "filename": part["filename"],
                        "content_type": part["content_type"],
                        "data": bytes(part["content"]).decode(),
                    } for part in upload["uploads"][0]["parts"]]}
    return {
        "result": result, "post_call": False, "post_matches": True, "request": observed,
    }
