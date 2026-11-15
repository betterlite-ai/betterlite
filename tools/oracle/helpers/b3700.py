import asyncio
import tempfile
from collections.abc import AsyncGenerator, AsyncIterator
from contextlib import aclosing
from pathlib import Path
from typing import Final, Literal, NotRequired, Protocol, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.litellm_content_filter.content_filter import (
    ContentFilterGuardrail,
)
from litellm.types.guardrails import BlockedWord, ContentFilterPattern, GuardrailEventHooks
from litellm.types.proxy.guardrails.guardrail_hooks.litellm_content_filter import (
    ContentFilterCategoryConfig,
)
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException

GUARDRAIL_NAME: Final = "stream-filter"
RECORDS_KEY: Final = "standard_logging_guardrail_information"
TIMING_FIELDS: Final = frozenset({"start_time", "end_time", "duration"})


class CategoryInput(TypedDict):
    category: str
    action: Literal["BLOCK", "MASK"]
    severity_threshold: Literal["high", "medium", "low"]
    file_text: str


class Input(TypedDict):
    patterns: list[ContentFilterPattern]
    blocked_words: list[BlockedWord]
    categories: list[CategoryInput]
    guardrail_id: str | None
    policy_template: str | None
    pattern_redaction_format: str | None
    keyword_redaction_tag: str | None
    items: list[Json]
    close_after: int | None


class DeltaDump(TypedDict):
    content: NotRequired[Json]


class ChoiceDump(TypedDict):
    index: NotRequired[Json]
    finish_reason: NotRequired[Json]
    delta: DeltaDump


class ChunkDump(TypedDict):
    id: Json
    choices: list[ChoiceDump]


class StreamHook(Protocol):
    def async_post_call_streaming_iterator_hook(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        response: AsyncIterator[ModelResponseStream],
        request_data: dict[str, Json],
    ) -> AsyncGenerator[ModelResponseStream, None]: ...


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)
CHUNK: Final[TypeAdapter[ModelResponseStream]] = TypeAdapter(ModelResponseStream)
DUMP: Final[TypeAdapter[ChunkDump]] = TypeAdapter(ChunkDump)
JSON: Final[TypeAdapter[Json]] = TypeAdapter(Json)


def categories(inputs: list[CategoryInput], folder: Path) -> list[ContentFilterCategoryConfig]:
    configs: list[ContentFilterCategoryConfig] = []
    for position, category in enumerate(inputs):
        path = folder / f"category_{position}.yaml"
        _ = path.write_text(category["file_text"], encoding="utf-8")
        configs.append(
            ContentFilterCategoryConfig(
                category=category["category"],
                action=category["action"],
                severity_threshold=category["severity_threshold"],
                category_file=str(path),
            )
        )
    return configs


def guardrail(request: Input, folder: Path) -> ContentFilterGuardrail:
    return ContentFilterGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        guardrail_id=request["guardrail_id"],
        policy_template=request["policy_template"],
        patterns=request["patterns"],
        blocked_words=request["blocked_words"],
        event_hook=GuardrailEventHooks.post_call,
        pattern_redaction_format=request["pattern_redaction_format"],
        keyword_redaction_tag=request["keyword_redaction_tag"],
        categories=categories(request["categories"], folder),
    )


async def feed(items: list[ModelResponseStream]) -> AsyncIterator[ModelResponseStream]:
    for item in items:
        yield item


def project(item: ModelResponseStream) -> Json:
    dump = DUMP.validate_json(item.model_dump_json())
    choices: list[Json] = [
        {
            "index": choice.get("index"),
            "finish_reason": choice.get("finish_reason"),
            "content": choice["delta"].get("content"),
        }
        for choice in dump["choices"]
    ]
    return {"id": dump["id"], "choices": choices}


def error_tag(error: StarletteHTTPException) -> Json:
    detail = clean_value(JSON.validate_python(error.detail))
    return {"status": error.status_code, "detail": detail}


def clean_fields(fields: dict[str, Json]) -> dict[str, Json]:
    cleaned: dict[str, Json] = {}
    for name, item in fields.items():
        kept = clean_value(item)
        if kept is not None:
            cleaned[name] = kept
    return cleaned


def clean_value(value: Json) -> Json | None:
    match value:
        case dict() as fields:
            return clean_fields(fields)
        case list() as items:
            return [kept for item in items if (kept := clean_value(item)) is not None]
        case None | bool() | int() | float() | str():
            return value


def visible_record(record: dict[str, Json]) -> Json:
    kept = {name: item for name, item in record.items() if name not in TIMING_FIELDS}
    return clean_fields(kept)


def oldest_record(value: Json) -> Json:
    match value:
        case [dict() as record, *_]:
            return visible_record(record)
        case None | bool() | int() | float() | str() | list() | dict():
            return None


def first_record(value: Json) -> Json:
    match value:
        case dict() as bucket:
            return oldest_record(bucket.get(RECORDS_KEY))
        case None | bool() | int() | float() | str() | list():
            return None


async def run_stream(streamer: StreamHook, request: Input) -> Json:
    request_data: dict[str, Json] = {}
    items = [CHUNK.validate_python(item) for item in request["items"]]
    chunks: list[Json] = []
    error: Json = None
    streamed = streamer.async_post_call_streaming_iterator_hook(
        UserAPIKeyAuth(), feed(items), request_data
    )
    try:
        async with aclosing(streamed) as stream:
            async for item in stream:
                chunks.append(project(item))
                if request["close_after"] is not None and len(chunks) >= request["close_after"]:
                    break
    except StarletteHTTPException as raised:
        error = error_tag(raised)
    record = first_record(request_data.get("metadata"))
    return {"chunks": chunks, "record": record, "error": error}


def run(input_value: dict[str, Json]) -> Json:
    request = INPUT.validate_python(input_value)
    with tempfile.TemporaryDirectory() as folder:
        return asyncio.run(run_stream(guardrail(request, Path(folder)), request))
