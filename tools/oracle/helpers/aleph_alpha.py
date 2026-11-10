import json
import os
import time

os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable, Generator, Iterator
from contextlib import ExitStack, contextmanager
from typing import TYPE_CHECKING, Protocol, TypeAlias, cast
from unittest.mock import patch
from urllib.parse import urlsplit

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.deprecated_providers import aleph_alpha
from litellm.llms.deprecated_providers.aleph_alpha import AlephAlphaConfig
from litellm.main import _get_encoding  # pyright: ignore[reportPrivateUsage]
from litellm.types.utils import ModelResponse

if TYPE_CHECKING:
    import tiktoken

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

FIELDS = frozenset(
    {
        "model",
        "messages",
        "api_base",
        "api_key",
        "optional_params",
        "defaults",
        "max_tokens",
        "created",
        "reply",
    }
)
HEADER_NAMES = ("accept", "content-type", "authorization")
COMPLETION = cast("Callable[..., object]", aleph_alpha.completion)


class ReplyMessage(Protocol):
    content: str | None


class Choice(Protocol):
    index: int
    finish_reason: str
    message: ReplyMessage


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class Completion(Protocol):
    choices: list[Choice]
    created: int
    model: str
    usage: Usage


class RecordingEncoding:
    def __init__(self) -> None:
        self.calls: list[Json] = []
        self.encoding = cast("tiktoken.Encoding", _get_encoding())

    def encode(self, text: str) -> list[int]:
        self.calls.append(text)
        return self.encoding.encode(text)


class RecordingTransport:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.calls: list[Object] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(
            {
                "url": str(request.url),
                "headers": {name.lower(): value for name, value in request.headers.items()},
                "body": cast("Json", json.loads(request.content)),
            }
        )
        return httpx.Response(self.status, content=self.body.encode(), request=request)


class RecordingLogging:
    def __init__(self) -> None:
        self.calls: list[Json] = []

    def pre_call(self, **params: Json) -> None:
        self.calls.append(_phase("pre_call", params))

    def post_call(self, **params: Json) -> None:
        self.calls.append(_phase("post_call", params))


def _phase(name: str, params: dict[str, Json]) -> Json:
    return {
        "phase": name,
        "input": params.get("input"),
        "api_key": params.get("api_key") is not None,
    }


def verbose(*args: Json, **kwargs: Json) -> None:
    del args, kwargs


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_text_or_none(value: Json) -> str | None:
    return None if value is None else expect_text(value)


def expect_int(value: Json) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("expected an integer")
    return value


@contextmanager
def patched_config(max_tokens: int, defaults: Object) -> Generator[None]:
    with ExitStack() as stack:
        stack.enter_context(patch.object(AlephAlphaConfig, "maximum_tokens", max_tokens))
        for name, value in defaults.items():
            stack.enter_context(patch.object(AlephAlphaConfig, name, value))
        yield


def status_of(error: Exception) -> Json:
    carried: object = getattr(error, "status_code", None)
    return carried if isinstance(carried, int) and not isinstance(carried, bool) else None


def project(result: Completion) -> Json:
    choices: list[Json] = []
    for choice in result.choices:
        native = cast("Object | None", getattr(choice, "provider_specific_fields", None))
        choices.append(
            {
                "index": choice.index,
                "content": choice.message.content,
                "finish_reason": choice.finish_reason,
                "native_finish_reason": None
                if native is None
                else native.get("native_finish_reason"),
            }
        )
    return {
        "choices": choices,
        "created": result.created,
        "model": result.model,
        "usage": {
            "prompt_tokens": result.usage.prompt_tokens,
            "completion_tokens": result.usage.completion_tokens,
            "total_tokens": result.usage.total_tokens,
        },
    }


def request_of(call: Object | None) -> Object:
    if call is None:
        return {"path": None, "query": None, "headers": None, "body": None}
    target = urlsplit(expect_text(call["url"]))
    headers = expect_object(call["headers"])
    return {
        "path": target.path,
        "query": target.query,
        "headers": {name: headers[name] for name in HEADER_NAMES if name in headers},
        "body": call["body"],
    }


def run(input_value: dict[str, Json]) -> Json:
    if frozenset(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    model = expect_text(input_value["model"])
    messages = expect_list(input_value["messages"])
    api_base = expect_text(input_value["api_base"])
    api_key = expect_text_or_none(input_value["api_key"])
    optional_params = expect_object(input_value["optional_params"])
    defaults = expect_object(input_value["defaults"])
    max_tokens = expect_int(input_value["max_tokens"])
    created = expect_int(input_value["created"])
    reply_input = expect_object(input_value["reply"])
    status = expect_int(reply_input["status"])
    body = expect_text(reply_input["body"])
    stream = optional_params.get("stream") is True
    encoding = RecordingEncoding()
    transport = RecordingTransport(status, body)
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(transport.handle)))
    logging_obj = RecordingLogging()
    reply: Json = None
    lines: Json = None
    error_status: Json = None
    failed = False
    with (
        patched_config(max_tokens, defaults),
        patch.object(litellm, "module_level_client", client),
        patch.object(time, "time", return_value=float(created)),
    ):
        try:
            result = COMPLETION(
                model=model,
                messages=messages,
                api_base=api_base,
                model_response=ModelResponse(),
                print_verbose=verbose,
                encoding=encoding,
                api_key=api_key,
                logging_obj=logging_obj,
                optional_params=dict(optional_params),
                litellm_params={},
                logger_fn=None,
                default_max_tokens_to_sample=max_tokens,
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            failed = True
            error_status = status_of(error)
        else:
            if stream:
                lines = cast("Json", list(cast("Iterator[str]", result)))
            else:
                reply = project(cast("Completion", result))
    tokens: Json = [] if stream or failed else list(encoding.calls)
    output: Object = {
        "ok": not failed,
        "status": error_status,
        "reply": reply,
        "lines": lines,
        "tokens": tokens,
        "log": logging_obj.calls,
    }
    output.update(request_of(transport.calls[0] if transport.calls else None))
    return output
