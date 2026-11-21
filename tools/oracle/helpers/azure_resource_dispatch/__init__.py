from typing import Final, Literal, TypedDict

import httpx
import litellm
import openai
from pydantic import JsonValue, TypeAdapter

from azure_assistant_definitions import run as definitions_run
from azure_assistant_definitions.sdk import Input as DefinitionsInput
from azure_assistants import run as assistants_run
from azure_assistants.calls import Input as AssistantsInput
from azure_batches import run as batches_run
from azure_batches.contracts import Input as BatchesInput
from azure_files import Step as FileStep
from azure_files import run as files_run
from azure_fine_tuning import Input as FineTuningInput
from azure_fine_tuning import run as fine_tuning_run
from azure_message_attachments import run as messages_run
from azure_message_attachments.sdk import Input as MessagesInput
from azure_passthrough import run as passthrough_run
from azure_realtime import run as realtime_run
from azure_realtime.types import Input as RealtimeInput
from azure_resource_urls import Input as ResourceUrlsInput
from azure_resource_urls import run as resource_urls_run
from azure_responses import run as responses_run
from azure_responses.contracts import Call as ResponsesCall
from azure_search import run as search_run
from azure_search.surface import Input as SearchInput
from azure_thread_messages import Input as ThreadsInput
from azure_thread_messages import run as threads_run

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

FIELDS: Final = TypeAdapter(dict[str, JsonValue])
ROWS: Final = TypeAdapter(list[JsonValue])
STRING: Final = TypeAdapter(str)
BOOLEAN: Final = TypeAdapter(bool)
OPTIONAL_NUMBER: Final = TypeAdapter(int | None)
VALUE: Final = TypeAdapter(JsonValue)

FILES_IN: Final = TypeAdapter(dict[str, list[FileStep]])
BATCHES_IN: Final = TypeAdapter(BatchesInput)
FINE_TUNING_IN: Final = TypeAdapter(FineTuningInput)
DEFINITIONS_IN: Final = TypeAdapter(DefinitionsInput)
THREADS_IN: Final = TypeAdapter(ThreadsInput)
MESSAGES_IN: Final = TypeAdapter(MessagesInput)
ASSISTANTS_IN: Final = TypeAdapter(AssistantsInput)
RESPONSES_IN: Final = TypeAdapter(ResponsesCall)
SEARCH_IN: Final = TypeAdapter(SearchInput)
REALTIME_IN: Final = TypeAdapter(RealtimeInput)
RESOURCE_URLS_IN: Final = TypeAdapter(ResourceUrlsInput)

FILE_ACTIONS: Final = {
    "file_create": "create",
    "file_content": "content",
    "file_retrieve": "retrieve",
    "file_list": "list",
    "file_delete": "delete",
}
BATCH_ACTIONS: Final = {
    "batch_create": "create",
    "batch_retrieve": "retrieve",
    "batch_cancel": "cancel",
    "batch_list": "list",
}
BATCH_CREATE: Final = '{"completion_window":"24h","endpoint":"/v1/embeddings","input_file_id":"file-1"}'
COMPLETED_RUN: Final = (
    '{"id":"run_1","object":"thread.run","created_at":1700000000,"completed_at":1700000005,'
    '"assistant_id":"asst_1","thread_id":"thread_1","status":"completed","instructions":"base",'
    '"model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}'
)


class Input(TypedDict):
    case: str


class FilesBatchesCase(TypedDict):
    case: Literal["files_batches"]
    steps: list[JsonValue]


class FineTuningCase(TypedDict):
    case: Literal["fine_tuning"]
    steps: list[JsonValue]


class AssistantThreadRunCase(TypedDict):
    case: Literal["assistant_thread_run"]
    steps: list[JsonValue]


class ResponsesSearchCase(TypedDict):
    case: Literal["responses_search"]
    steps: list[JsonValue]


class RealtimeCase(TypedDict):
    case: Literal["realtime"]
    model: str
    scope_headers: list[JsonValue]
    ssl_verification: str
    max_message_size_bytes: int
    azure_api_base: str | None
    azure_api_key: str | None
    azure_api_version: str | None
    queries: JsonValue
    protocols: JsonValue
    failures: JsonValue
    http_setups: JsonValue


class ContainerVideoVectorCase(TypedDict):
    case: Literal["container_video_vector"]
    urls: list[JsonValue]
    video: list[JsonValue]


class PassthroughCase(TypedDict):
    case: Literal["passthrough"]
    streaming_steps: list[JsonValue]
    url_steps: list[JsonValue]


FILES_CASE: Final = TypeAdapter(FilesBatchesCase)
FINE_TUNING_CASE: Final = TypeAdapter(FineTuningCase)
ASSISTANT_CASE: Final = TypeAdapter(AssistantThreadRunCase)
RESPONSES_CASE: Final = TypeAdapter(ResponsesSearchCase)
REALTIME_CASE: Final = TypeAdapter(RealtimeCase)
CONTAINER_CASE: Final = TypeAdapter(ContainerVideoVectorCase)
PASSTHROUGH_CASE: Final = TypeAdapter(PassthroughCase)


def object_of(value: object) -> dict[str, JsonValue]:
    return FIELDS.validate_python(value)


def rows_of(value: object) -> list[JsonValue]:
    return ROWS.validate_python(value)


def text_of(value: object) -> str:
    return STRING.validate_python(value)


def first_row(value: object) -> dict[str, JsonValue]:
    return object_of(rows_of(value)[0])


def refusal(status: int) -> openai.APIStatusError:
    request = httpx.Request("POST", "https://fixture.test")
    response = httpx.Response(status, request=request)
    body: dict[str, JsonValue] = {"message": "the fixture failed"}
    return openai.APIStatusError("the fixture failed", response=response, body=body)


def raise_raised(raised: JsonValue) -> None:
    fields = object_of(raised)
    status = OPTIONAL_NUMBER.validate_python(fields.get("status"))
    if status is not None:
        raise refusal(status)
    raise ValueError("the fixture raised an error")


def raise_error(row: dict[str, JsonValue]) -> None:
    error = row.get("error")
    if error is None:
        return
    raise_raised(error)


def arguments(step: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "api_key": step["api_key"],
        "api_base": step["api_base"],
        "api_version": step["api_version"],
        "timeout": 600.0,
        "max_retries": 0,
    }


def file_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    result = object_of(row["result"])
    if kind == "file_content":
        return {"status": result["status"], "length": len(text_of(result["body"]))}
    if kind == "file_list":
        return {"count": len(rows_of(result["data"]))}
    if kind == "file_delete":
        return {"deleted": result["deleted"]}
    return {"id": result["id"], "bytes": result["bytes"]}


def file_step(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "action": FILE_ACTIONS[kind],
        "args": arguments(step),
        "reply": {
            "status": step["status"],
            "headers": {"content-type": "application/json"},
            "body": text_of(step["body"]),
        },
    }
    if kind == "file_create":
        payload["content"] = "plain"
        payload["create_purpose"] = "batch"
    else:
        payload["file_id"] = text_of(step.get("target_id", ""))
    return payload


def batch_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    outcome = object_of(row["outcome"])
    returned = outcome.get("returned")
    if returned is None:
        raise_raised(outcome["raised"])
        return None
    batch = object_of(returned)
    if kind == "batch_list":
        more = batch.get("has_more")
        return {
            "count": len(rows_of(batch["data"])),
            "has_more": False if more is None else BOOLEAN.validate_python(more),
        }
    return {"id": batch["id"], "status": batch["status"]}


def batch_step(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    action = BATCH_ACTIONS[kind]
    payload: dict[str, JsonValue] = {
        "action": action,
        "args": arguments(step),
        "reply": {"status": step["status"], "body": VALUE.validate_json(text_of(step["body"]))},
    }
    if action == "create":
        payload["data"] = VALUE.validate_json(BATCH_CREATE)
    elif action == "list":
        payload["after"] = step.get("after")
        payload["limit"] = step.get("limit")
    else:
        payload["query"] = {"batch_id": text_of(step.get("target_id", ""))}
    return payload


def files_batches(case: FilesBatchesCase) -> JsonValue:
    answers: list[JsonValue] = []
    for item in case["steps"]:
        step = object_of(item)
        kind = text_of(step["kind"])
        if kind in FILE_ACTIONS:
            answer = files_run(FILES_IN.validate_python({"steps": [file_step(kind, step)]}))
            row = first_row(object_of(answer)["steps"])
            raise_error(row)
            value = file_value(kind, row)
        else:
            answer = batches_run(BATCHES_IN.validate_python({"steps": [batch_step(kind, step)]}))
            row = first_row(object_of(answer)["steps"])
            value = batch_value(kind, row)
        answers.append({"kind": kind, "value": value})
    return {"steps": answers}


def fine_tuning_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    outcome = object_of(row["outcome"])
    returned = outcome.get("returned")
    if returned is None:
        raise_raised(outcome["raised"])
        return None
    job = object_of(returned)
    if kind == "list":
        more = job.get("has_more")
        return {
            "count": len(rows_of(job["data"])),
            "has_more": False if more is None else BOOLEAN.validate_python(more),
        }
    if kind == "create":
        data = object_of(row["data"])
        extra = object_of(data["extra_body"])
        return {"id": job["id"], "status": job["status"], "training_type": extra["trainingType"]}
    return {"id": job["id"], "status": job["status"]}


def fine_tuning_step(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "action": kind,
        "args": arguments(step),
        "reply": {"status": step["status"], "body": VALUE.validate_json(text_of(step["body"]))},
    }
    if kind == "create":
        payload["data"] = {
            "model": "gpt-4o-mini",
            "training_file": "file_a",
        }
    elif kind == "list":
        payload["after"] = step.get("after")
        payload["limit"] = step.get("limit")
    else:
        payload["job_id"] = text_of(step.get("job_id", ""))
    return payload


def fine_tuning(case: FineTuningCase) -> JsonValue:
    answers: list[JsonValue] = []
    for item in case["steps"]:
        step = object_of(item)
        kind = text_of(step["kind"])
        answer = fine_tuning_run(FINE_TUNING_IN.validate_python({"steps": [fine_tuning_step(kind, step)]}))
        row = first_row(object_of(answer)["steps"])
        answers.append({"kind": kind, "value": fine_tuning_value(kind, row)})
    return {"steps": answers}


def assistant_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    error = row.get("error")
    if error is not None:
        raise_raised(error)
        return None
    result = object_of(row["result"])
    if kind == "definitions_list" or kind == "message_list":
        return {"count": len(rows_of(result["data"]))}
    if kind == "definitions_delete":
        return {"deleted": result["deleted"]}
    if kind == "run_poll":
        return {"status": result["status"]}
    return {"id": result["id"]}


def definition_input(step: dict[str, JsonValue], kind: str) -> JsonValue:
    payload = arguments(step)
    if kind == "definitions_create":
        payload["data"] = {"model": "gpt-4o"}
    if kind == "definitions_delete":
        payload["assistant_id"] = "asst_1"
    return payload


def assistant_step(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    if kind in {"definitions_create", "definitions_list", "definitions_delete"}:
        key = kind.removeprefix("definitions_")
        variant = {"create": "create_assistant", "list": "list_assistants", "delete": "delete_assistant"}[key]
        answer = definitions_run(DEFINITIONS_IN.validate_python({variant: [definition_input(step, kind)]}))
        return first_row(object_of(answer)["steps"])
    if kind == "thread_create":
        payload = arguments(step)
        payload["messages"] = [{"role": "user", "content": "hello"}]
        answer = threads_run(THREADS_IN.validate_python({"thread_create": [payload]}))
        return first_row(object_of(answer)["steps"])
    if kind == "message_create":
        payload = arguments(step)
        payload["thread_id"] = "thread_1"
        payload["data"] = {"role": "user", "content": "hello"}
        answer = messages_run(MESSAGES_IN.validate_python({"append_message": [payload]}))
        return first_row(object_of(answer)["steps"])
    if kind == "message_list":
        payload = arguments(step)
        payload["thread_id"] = "thread_1"
        answer = messages_run(MESSAGES_IN.validate_python({"list_messages": [payload]}))
        return first_row(object_of(answer)["steps"])
    payload = arguments(step)
    payload["run"] = {
        "thread_id": "thread_1",
        "assistant_id": "asst_1",
        "model": "gpt-4o-mini",
        "instructions": "base",
    }
    payload["replies"] = [{"status": 200, "body": COMPLETED_RUN, "headers": {}}]
    answer = assistants_run(ASSISTANTS_IN.validate_python({"run_poll": [payload]}))
    return first_row(object_of(answer)["steps"])


def assistant_thread_run(case: AssistantThreadRunCase) -> JsonValue:
    answers: list[JsonValue] = []
    for item in case["steps"]:
        step = object_of(item)
        kind = text_of(step["kind"])
        row = assistant_step(kind, step)
        answers.append({"kind": kind, "value": assistant_value(kind, row)})
    return {"steps": answers}


def projected_of(row: dict[str, JsonValue]) -> JsonValue:
    projected: dict[str, JsonValue] = {}
    for name in ("model", "input", "tools"):
        if name in row:
            projected[name] = row[name]
    return projected


def responses_url(step: dict[str, JsonValue]) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "function": "base_and_api_version",
        "default_api_version": "2025-04-01-preview",
        "steps": [
            {
                "api_base": step["api_base"],
                "api_version": step.get("api_version"),
                "headers": {},
                "api_key": "key-1",
            }
        ],
    }
    row = first_row(object_of(responses_run(RESPONSES_IN.validate_python(payload)))["steps"])
    if not BOOLEAN.validate_python(row["ok"]):
        raise ValueError("the fixture built no responses url")
    return {"url": row["url"]}


def responses_transform(step: dict[str, JsonValue]) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "function": "strip_model_prefix",
        "steps": [{"model": step["model"], "input": step["input"]}],
    }
    row = first_row(object_of(responses_run(RESPONSES_IN.validate_python(payload)))["steps"])
    return projected_of(row)


def responses_resource(step: dict[str, JsonValue], kind: str) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "endpoint": text_of(step["endpoint"]),
        "api_base": step["api_base"],
        "response_id": step["response_id"],
    }
    if kind == "responses_input_items":
        payload["endpoint"] = "list"
        for name in ("after", "before", "include", "limit", "order"):
            if name in step:
                payload[name] = step[name]
    call: dict[str, JsonValue] = {
        "function": "resource_id_before_query",
        "steps": [payload],
    }
    row = first_row(object_of(responses_run(RESPONSES_IN.validate_python(call)))["steps"])
    if not BOOLEAN.validate_python(row["ok"]):
        raise ValueError("the fixture built no responses request")
    return {"url": row["url"], "data": row["data"]}


def search_url(step: dict[str, JsonValue]) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "settings": {"urls": [{"environment": {}, "api_base": step["api_base"]}], "requests": []}
    }
    row = first_row(object_of(search_run(SEARCH_IN.validate_python(payload)))["urls"])
    if not BOOLEAN.validate_python(row["ok"]):
        raise ValueError("the fixture built no search url")
    return {"url": row["url"]}


def search_request(step: dict[str, JsonValue]) -> JsonValue:
    model = text_of(step["model"])
    payload: dict[str, JsonValue] = {
        "request": {
            "scenarios": [
                {
                    "environment": {"BING_GROUNDING_MODEL": model},
                    "query": text_of(step["query"]),
                }
            ]
        }
    }
    row = first_row(object_of(search_run(SEARCH_IN.validate_python(payload)))["requests"])
    if not BOOLEAN.validate_python(row["ok"]):
        raise ValueError("the fixture built no search request")
    body = object_of(row["body"])
    return {"model": body["model"], "input": body["input"], "tools": body["tools"]}


def responses_search(case: ResponsesSearchCase) -> JsonValue:
    answers: list[JsonValue] = []
    for item in case["steps"]:
        step = object_of(item)
        kind = text_of(step["kind"])
        if kind == "responses_url":
            value = responses_url(step)
        elif kind == "responses_transform":
            value = {"projected": responses_transform(step)}
        elif kind == "search_url":
            value = search_url(step)
        elif kind == "search_request":
            value = search_request(step)
        else:
            value = responses_resource(step, kind)
        answers.append({"kind": kind, "value": value})
    return {"steps": answers}


def realtime_payload(case: RealtimeCase, kind: str) -> RealtimeInput:
    payload: dict[str, JsonValue] = {
        "kind": kind,
        "model": case["model"],
        "scope_headers": case["scope_headers"],
        "ssl_verification": case["ssl_verification"],
        "max_message_size_bytes": case["max_message_size_bytes"],
        "azure_api_base": case["azure_api_base"],
        "azure_api_key": case["azure_api_key"],
        "azure_api_version": case["azure_api_version"],
        "queries": case["queries"],
        "protocols": case["protocols"],
        "failures": case["failures"],
        "http_setups": case["http_setups"],
    }
    return REALTIME_IN.validate_python(payload)


def realtime(case: RealtimeCase) -> JsonValue:
    connects = object_of(realtime_run(realtime_payload(case, "connect")))["connects"]
    http = object_of(realtime_run(realtime_payload(case, "http")))["http"]
    return {"connects": connects, "http": http}


def container_video_vector(case: ContainerVideoVectorCase) -> JsonValue:
    urls_payload: dict[str, JsonValue] = {"container_api_version_query": case["urls"]}
    urls = object_of(resource_urls_run(RESOURCE_URLS_IN.validate_python(urls_payload)))["steps"]
    video_payload: dict[str, JsonValue] = {"video_supported_fields": case["video"]}
    video = object_of(resource_urls_run(RESOURCE_URLS_IN.validate_python(video_payload)))["steps"]
    return {"urls": urls, "video": video}


def passthrough(case: PassthroughCase) -> JsonValue:
    inference_payload: dict[str, JsonValue] = {
        "function": "body_model_inference_endpoint",
        "inference_steps": case["streaming_steps"],
    }
    inference = object_of(passthrough_run(inference_payload))["steps"]
    streaming = [
        object_of(row)["streaming"] for row in rows_of(inference)
    ]
    removal_payload: dict[str, JsonValue] = {
        "function": "api_version_removal",
        "removals": [],
        "url_steps": case["url_steps"],
    }
    urls = object_of(passthrough_run(removal_payload))["steps"]
    return {"streaming": streaming, "urls": urls}


def run(input_value: Input) -> JsonValue:
    fields = object_of(input_value)
    case = text_of(fields["case"])
    if case == "files_batches":
        return files_batches(FILES_CASE.validate_python(input_value))
    if case == "fine_tuning":
        return fine_tuning(FINE_TUNING_CASE.validate_python(input_value))
    if case == "assistant_thread_run":
        return assistant_thread_run(ASSISTANT_CASE.validate_python(input_value))
    if case == "responses_search":
        return responses_search(RESPONSES_CASE.validate_python(input_value))
    if case == "realtime":
        return realtime(REALTIME_CASE.validate_python(input_value))
    if case == "container_video_vector":
        return container_video_vector(CONTAINER_CASE.validate_python(input_value))
    if case == "passthrough":
        return passthrough(PASSTHROUGH_CASE.validate_python(input_value))
    raise ValueError(f"unknown azure resource dispatch case {case}")
