from collections.abc import Callable
from typing import Final, Literal, TypedDict

import httpx
import openai
from pydantic import JsonValue, TypeAdapter

from azure_assistant_definitions.sdk import Input as DefinitionsInput
from azure_assistants.calls import Input as AssistantsInput
from azure_batches.contracts import Input as BatchesInput
from azure_files import Step as FileStep
from azure_fine_tuning import Input as FineTuningInput
from azure_message_attachments.sdk import Input as MessagesInput
from azure_realtime.types import Input as RealtimeInput
from azure_responses.contracts import Call as ResponsesCall
from azure_search.surface import Input as SearchInput
from azure_thread_messages import Input as ThreadsInput

FIELDS: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
ROWS: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])
STRING: Final[TypeAdapter[str]] = TypeAdapter(str)
BOOLEAN: Final[TypeAdapter[bool]] = TypeAdapter(bool)
OPTIONAL_NUMBER: Final[TypeAdapter[int | None]] = TypeAdapter(int | None)
VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

FILES_IN: Final[TypeAdapter[dict[str, list[FileStep]]]] = TypeAdapter(
    dict[str, list[FileStep]]
)
BATCHES_IN: Final[TypeAdapter[BatchesInput]] = TypeAdapter(BatchesInput)
FINE_TUNING_IN: Final[TypeAdapter[FineTuningInput]] = TypeAdapter(FineTuningInput)
DEFINITIONS_IN: Final[TypeAdapter[DefinitionsInput]] = TypeAdapter(DefinitionsInput)
THREADS_IN: Final[TypeAdapter[ThreadsInput]] = TypeAdapter(ThreadsInput)
MESSAGES_IN: Final[TypeAdapter[MessagesInput]] = TypeAdapter(MessagesInput)
ASSISTANTS_IN: Final[TypeAdapter[AssistantsInput]] = TypeAdapter(AssistantsInput)
RESPONSES_IN: Final[TypeAdapter[ResponsesCall]] = TypeAdapter(ResponsesCall)
SEARCH_IN: Final[TypeAdapter[SearchInput]] = TypeAdapter(SearchInput)
REALTIME_IN: Final[TypeAdapter[RealtimeInput]] = TypeAdapter(RealtimeInput)

FILE_ACTIONS: Final[dict[str, str]] = {
    "file_create": "create",
    "file_content": "content",
    "file_retrieve": "retrieve",
    "file_list": "list",
    "file_delete": "delete",
}
BATCH_ACTIONS: Final[dict[str, str]] = {
    "batch_create": "create",
    "batch_retrieve": "retrieve",
    "batch_cancel": "cancel",
    "batch_list": "list",
}
BATCH_CREATE: Final[str] = (
    '{"completion_window":"24h","endpoint":"/v1/embeddings","input_file_id":"file-1"}'
)
COMPLETED_RUN: Final[str] = (
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


FILES_CASE: Final[TypeAdapter[FilesBatchesCase]] = TypeAdapter(FilesBatchesCase)
FINE_TUNING_CASE: Final[TypeAdapter[FineTuningCase]] = TypeAdapter(FineTuningCase)
ASSISTANT_CASE: Final[TypeAdapter[AssistantThreadRunCase]] = TypeAdapter(
    AssistantThreadRunCase
)
RESPONSES_CASE: Final[TypeAdapter[ResponsesSearchCase]] = TypeAdapter(
    ResponsesSearchCase
)
REALTIME_CASE: Final[TypeAdapter[RealtimeCase]] = TypeAdapter(RealtimeCase)
CONTAINER_CASE: Final[TypeAdapter[ContainerVideoVectorCase]] = TypeAdapter(
    ContainerVideoVectorCase
)
PASSTHROUGH_CASE: Final[TypeAdapter[PassthroughCase]] = TypeAdapter(PassthroughCase)

Producer = Callable[[str, dict[str, JsonValue]], JsonValue]


def refusal(status: int) -> openai.APIStatusError:
    request = httpx.Request("POST", "https://fixture.test")
    response = httpx.Response(status, request=request)
    body: dict[str, JsonValue] = {"message": "the fixture failed"}
    return openai.APIStatusError("the fixture failed", response=response, body=body)


def raise_raised(raised: JsonValue) -> None:
    fields = FIELDS.validate_python(raised)
    status = OPTIONAL_NUMBER.validate_python(fields.get("status"))
    if status is not None:
        raise refusal(status)
    raise ValueError("the fixture raised an error")


def raise_error(row: dict[str, JsonValue]) -> None:
    error = row.get("error")
    if error is not None:
        raise_raised(error)


def arguments(step: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "api_key": step["api_key"],
        "api_base": step["api_base"],
        "api_version": step["api_version"],
        "timeout": 600.0,
        "max_retries": 0,
        "organization": None,
    }


def first_step(answer: JsonValue, key: str) -> dict[str, JsonValue]:
    rows = ROWS.validate_python(FIELDS.validate_python(answer)[key])
    return FIELDS.validate_python(rows[0])


def stepped(steps: list[JsonValue], produce: Producer) -> JsonValue:
    answers: list[JsonValue] = []
    for item in steps:
        step = FIELDS.validate_python(item)
        kind = STRING.validate_python(step["kind"])
        answers.append({"kind": kind, "value": produce(kind, step)})
    return {"steps": answers}


def reply_payload(
    action: str,
    step: dict[str, JsonValue],
    reply: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    payload: dict[str, JsonValue] = {
        "action": action,
        "args": arguments(step),
        "reply": reply,
    }
    return payload


def assistant_reply(step: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "status": step["status"],
        "body": STRING.validate_python(step["body"]),
    }


def paged(payload: dict[str, JsonValue], step: dict[str, JsonValue]) -> None:
    if "after" in step:
        payload["after"] = step["after"]
    if "limit" in step:
        payload["limit"] = step["limit"]


def returned_of(row: dict[str, JsonValue]) -> dict[str, JsonValue]:
    outcome = FIELDS.validate_python(row["outcome"])
    value = outcome.get("returned")
    if value is None:
        raise_raised(outcome["raised"])
    return FIELDS.validate_python(value)


def page_value(value: dict[str, JsonValue]) -> JsonValue:
    more = value.get("has_more")
    count = len(ROWS.validate_python(value["data"]))
    flag = False if more is None else BOOLEAN.validate_python(more)
    return {"count": count, "has_more": flag}


def checked_url(row: dict[str, JsonValue]) -> JsonValue:
    if not BOOLEAN.validate_python(row["ok"]):
        raise ValueError("the fixture built no request")
    return {"url": row["url"]}
