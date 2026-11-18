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


def count(supplied: Input) -> Observation:
    payload: dict[str, JsonValue] = {
        "function": "handle", "cases": [{
            "model": supplied["model"], "messages": JSON.validate_python(supplied["messages"]),
            "api_key": supplied["api_key"], "api_base": supplied["api_base"],
            "system": supplied["system"], "tools": JSON.validate_python(supplied["tools"]),
            "optional_params": supplied["optional_params"],
            "reply": {"body": supplied["response"], "status": 200},
        }],
    }
    answer = TypeAdapter(CountOutput).validate_python(anthropic_count_tokens.run(payload))
    call = answer["calls"][0]
    request = call["request"]
    headers = {name: value for name, value in request["headers"].items() if value is not None}
    key = headers["x-api-key"]
    headers["x-api-key"] = "matched" if key == supplied["expected_key"] else "mismatch"
    observed: Recorded = {
        "method": "POST", "url": request["url"], "query": [],
        "headers": headers, "body": request["body"],
    }
    return {
        "result": call["response"]["value"], "post_call": False,
        "post_matches": True, "request": observed,
    }


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


def batch(supplied: Input) -> Observation:
    if supplied["action"] == "batch_create":
        anthropic_batches.run({"create_requests": {"environment": {}, "url_bases": []}})
        return {
            "result": {"error": True, "status": None}, "post_call": False,
            "post_matches": True, "request": None,
        }
    answer = TypeAdapter(BatchOutput).validate_python(anthropic_batches.run({"retrieve": {
        "now": 1_700_000_000, "scenarios": [{
            "batch_id": supplied["id"], "api_base": supplied["api_base"],
            "api_key": supplied["api_key"], "environment": {},
            "body": supplied["response"], "sync": False,
        }],
    }}))
    scenario = answer["scenarios"][0]
    result = TypeAdapter(Batch).validate_json(scenario["batch"].model_dump_json())
    request = scenario["requests"][0]
    headers = request["headers"]
    headers[request["auth_header"]] = "matched"
    observed: Recorded = {
        "method": request["method"], "url": request["url"],
        "query": [], "headers": headers, "body": None,
    }
    return {
        "result": JSON.validate_python(result), "post_call": False,
        "post_matches": True, "request": observed,
    }


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


def files(supplied: Input) -> Observation:
    auth = TypeAdapter(FileHeaders).validate_python(anthropic_files.run({"file_beta_header": {
        "scenarios": [{"api_key": supplied["api_key"], "expected_auth": supplied["expected_key"],
            "caller_headers": [[name, value] for name, value in supplied["headers"].items()],
            "url_bases": [supplied["api_base"]],
            "error": {"status": 400, "message": "fixture", "headers": []},
        }],
    }}))["scenarios"][0]
    headers = auth["headers"]
    headers["x-api-key"] = "matched" if headers["x-api-key"] == supplied["expected_key"] else "mismatch"
    request: Recorded = {
        "method": "GET", "url": auth["urls"][0], "query": [], "headers": headers, "body": None,
    }
    result: JsonValue
    if supplied["action"] == "list":
        reply = TypeAdapter(FileList).validate_python(anthropic_files.run({"file_list_pagination": {
            "now": 1_700_000_000, "purposes": [supplied["purpose"]],
            "params_api_base": supplied["api_base"],
            "body": JSON.validate_json(supplied["response"]),
        }}))
        request["url"], request["query"] = reply["url"], reply["queries"][0]
        result = JSON.validate_python(reply["files"])
    elif supplied["action"] in {"content", "batch_results"}:
        content = TypeAdapter(Content).validate_python(anthropic_files.run({"file_content_bytes": {
            "file_ids": [supplied["id"]], "params_api_base": supplied["api_base"],
            "status": 200, "body": list(supplied["response"].encode()), "headers": [],
            "error": {"status": 400, "message": "fixture", "headers": []},
        }}))
        request["url"] = content["urls"][0]["url"]
        result = bytes(content["result"]["body"]).decode()
    else:
        body = JSON.validate_json(supplied["response"])
        route = TypeAdapter(FileDelete).validate_python(anthropic_files.run({"file_delete_shape": {
            "file_ids": [supplied["id"]], "params_api_base": supplied["api_base"], "bodies": [body],
        }}))
        if supplied["action"] == "delete":
            request["method"], request["url"] = "DELETE", route["delete"][0]["url"]
            result = JSON.validate_python(route["responses"][0])
        else:
            replies = TypeAdapter(FileReplies).validate_python(anthropic_files.run({
                "native_file_object": {"now": 1_700_000_000, "bodies": [body]},
            }))
            result = JSON.validate_python(replies["retrieve"][0])
            if supplied["action"] == "retrieve":
                request["url"] = route["retrieve"][0]["url"]
            else:
                upload = TypeAdapter(Uploads).validate_python(anthropic_files.run({
                    "file_upload_multipart": {
                    "now": 1_700_000_000, "uploads": [{
                        "data": supplied["file"], "filename": "input.jsonl",
                        "content_type": "application/jsonl", "purpose": supplied["purpose"],
                    }],
                }}))
                request["method"] = "POST"
                request["body"] = {"boundary": "oracle-boundary", "parts": [{
                    "name": part["name"], "filename": part["filename"],
                    "content_type": part["content_type"], "data": bytes(part["content"]).decode(),
                } for part in upload["uploads"][0]["parts"]]}
    return {
        "result": result, "post_call": False, "post_matches": True, "request": request,
    }
