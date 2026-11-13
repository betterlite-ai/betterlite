import asyncio
from collections.abc import Callable
from typing import Literal, NotRequired, TypedDict

import httpx
from fastapi import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.compresr.compresr import (  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/compresr/compresr.py#L109-L246
    CompresrGuardrail,
    _query_for_target,
    _render_tool_intent,
    _replace_text_in_content,
    _safe_int,
    _safe_response_text,
)
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
Messages = list[dict[str, object]]
Fields = dict[str, object]

Function = Literal[
    "validate_api_base",
    "query_for_target",
    "render_tool_intent",
    "fallback_query",
    "replace_text_in_content",
    "safe_int",
    "safe_response_text",
    "compress",
]


class Goal(TypedDict):
    context: str
    query: str


class Reply(TypedDict):
    status: int
    body: str
    corrupt_encoding: NotRequired[bool]


class Settings(TypedDict):
    api_base: str
    model: str
    target_compression_ratio: float
    coarse: bool
    dynamic: bool
    dynamic_min_ratio: float | None
    dynamic_max_ratio: float | None
    compression_params: Fields
    timeout: float | None
    fallback: Literal["fail_open", "fail_closed"]


class Input(TypedDict):
    function: Function
    messages: NotRequired[Messages]
    target_idx: NotRequired[int]
    fallback: NotRequired[str]
    function_call: NotRequired[Fields]
    content: NotRequired[JsonValue]
    new_text: NotRequired[str]
    value: NotRequired[JsonValue]
    text: NotRequired[str]
    limit: NotRequired[int]
    corrupt: NotRequired[bool]
    settings: NotRequired[Settings]
    goals: NotRequired[list[Goal]]
    reply: NotRequired[Reply]


class Recorder:
    def __init__(self, reply: Reply) -> None:
        self.paths: list[JsonValue] = []
        self.bodies: list[JsonValue] = []
        self.headers: list[JsonValue] = []
        self.reply = reply

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.paths.append(request.url.path)
        self.bodies.append(JSON.validate_json(request.content))
        content_type = request.headers.get("content-type")
        api_key = request.headers.get("x-api-key")
        self.headers.append(
            {
                "content_type": "" if content_type is None else content_type,
                "api_key": "" if api_key is None else api_key,
            }
        )
        corrupt = self.reply.get("corrupt_encoding")
        headers = {"content-encoding": "gzip"} if corrupt is True else {}
        return httpx.Response(
            self.reply["status"], headers=headers, content=self.reply["body"].encode()
        )


class Probe(CompresrGuardrail):
    def compress(self, goals: list[Goal], reply: Reply) -> JsonValue:
        recorder = Recorder(reply)
        self.async_handler.client = httpx.AsyncClient(
            transport=httpx.MockTransport(recorder.handle)
        )
        contexts = [goal["context"] for goal in goals]
        queries = [goal["query"] for goal in goals]
        try:
            results = asyncio.run(self._call_compress(contexts=contexts, queries=queries))
        except HTTPException as error:
            return {
                "path": recorder.paths,
                "body": recorder.bodies,
                "headers": recorder.headers,
                "outcome": "raised",
                "status": error.status_code,
            }
        if results is None:
            return {
                "path": recorder.paths,
                "body": recorder.bodies,
                "headers": recorder.headers,
                "outcome": "uncompressed",
            }
        return {
            "path": recorder.paths,
            "body": recorder.bodies,
            "headers": recorder.headers,
            "outcome": "results",
            "results": JSON.validate_python(results),
        }

    def fallback_of(self, messages: Messages) -> JsonValue:
        text, index = self._extract_fallback_query(messages)
        return {"query": text, "index": index}


def build(settings: Settings) -> Probe:
    return Probe(
        api_base=settings["api_base"],
        api_key="test-key",
        model=settings["model"],
        target_compression_ratio=settings["target_compression_ratio"],
        coarse=settings["coarse"],
        dynamic=settings["dynamic"],
        dynamic_min_ratio=settings["dynamic_min_ratio"],
        dynamic_max_ratio=settings["dynamic_max_ratio"],
        compression_params=settings["compression_params"],
        unreachable_fallback=settings["fallback"],
        timeout=settings["timeout"],
        enable_retrieval=False,
    )


def validate(input_value: Input) -> JsonValue:
    try:
        guardrail = build(input_value["settings"])
    except ValueError:
        return {"accepted": False}
    return {"accepted": True, "api_base": guardrail.compresr_api_base}


def intent(input_value: Input) -> JsonValue:
    rendered = _render_tool_intent(input_value["function_call"])
    return {"intent": rendered}


def query(input_value: Input) -> JsonValue:
    return {
        "query": _query_for_target(
            input_value["messages"], input_value["target_idx"], input_value["fallback"]
        )
    }


def rewrite(input_value: Input) -> JsonValue:
    replaced = _replace_text_in_content(input_value["content"], input_value["new_text"])
    return {"content": JSON.validate_python(replaced)}


def token_stat(input_value: Input) -> JsonValue:
    stat = _safe_int(input_value["value"])
    return {"value": stat}


def safe_text(input_value: Input) -> JsonValue:
    text = input_value["text"]
    corrupt = input_value["corrupt"]
    headers = {"content-encoding": "gzip"} if corrupt else {}
    response = httpx.Response(200, headers=headers, content=text.encode())
    return {"text": _safe_response_text(response, input_value["limit"])}


def compress(input_value: Input) -> JsonValue:
    guardrail = build(input_value["settings"])
    return guardrail.compress(input_value["goals"], input_value["reply"])


def fallback(input_value: Input) -> JsonValue:
    guardrail = build(input_value["settings"])
    return guardrail.fallback_of(input_value["messages"])


HANDLERS: dict[Function, Callable[[Input], JsonValue]] = {
    "validate_api_base": validate,
    "query_for_target": query,
    "render_tool_intent": intent,
    "fallback_query": fallback,
    "replace_text_in_content": rewrite,
    "safe_int": token_stat,
    "safe_response_text": safe_text,
    "compress": compress,
}


def run(input_value: Input) -> JsonValue:
    handler = HANDLERS[input_value["function"]]
    return handler(input_value)
