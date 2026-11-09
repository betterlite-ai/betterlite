import asyncio
import json
import time
from collections.abc import AsyncIterator, Callable, Generator, Iterator
from contextlib import contextmanager
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.replicate.chat import handler, transformation
from litellm.llms.replicate.chat.transformation import ReplicateConfig
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
FIXED_NOW = 1_700_000_000.0
DEFAULT_RETRIES = 5
DEFAULT_RETRY_DELAY = 1.0
DEFAULT_POLLING_DELAY = 0.5
DEFAULT_FIELDS = [
    "system_prompt",
    "max_new_tokens",
    "min_new_tokens",
    "temperature",
    "top_p",
    "top_k",
    "stop_sequences",
    "seed",
    "debug",
]

CONFIG = ReplicateConfig()
SUPPORTED = cast("Callable[..., list[str]]", CONFIG.get_supported_openai_params)
MAP_PARAMS = cast("Callable[..., Object]", CONFIG.map_openai_params)
COMPLETE_URL = cast("Callable[..., str]", CONFIG.get_complete_url)
VALIDATE = cast("Callable[..., Object]", CONFIG.validate_environment)
TRANSFORM_REQUEST = cast("Callable[..., Object]", CONFIG.transform_request)
TRANSFORM_RESPONSE = cast("Callable[..., object]", CONFIG.transform_response)
COMPLETION = cast("Callable[..., object]", handler.completion)
SYNC_STREAM = cast(
    "Callable[..., Iterator[Json]]", handler.handle_prediction_response_streaming
)
ASYNC_STREAM = cast(
    "Callable[..., AsyncIterator[Json]]",
    handler.async_handle_prediction_response_streaming,
)


class ChoiceMessage(Protocol):
    content: str | None


class Choice(Protocol):
    message: ChoiceMessage


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class Completion(Protocol):
    choices: list[Choice]
    created: int
    model: str
    usage: Usage


class Logging:
    def post_call(self, **params: Json) -> None:
        del params


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def optional_int(value: Json, default: int) -> int:
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("expected an integer")
    return value


def optional_float(value: Json, default: float) -> float:
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError("expected a number")
    return float(value)


def optional_text(value: Json) -> str | None:
    if value is None:
        return None
    return expect_text(value)


def headers_value(value: Json) -> dict[str, str]:
    return {
        expect_text(name): expect_text(entry)
        for name, entry in expect_object(value or {}).items()
    }


def failure_from(error: Exception) -> Object:
    carried = getattr(error, "status_code", None)
    status = int(carried) if isinstance(carried, int) and not isinstance(carried, bool) else None
    return {"ok": False, "status": status, "message": str(error)}


@contextmanager
def with_defaults(defaults: Object) -> Generator[None]:
    saved = {name: getattr(ReplicateConfig, name, None) for name in DEFAULT_FIELDS}
    for name in DEFAULT_FIELDS:
        value = defaults.get(name)
        if value is not None:
            setattr(ReplicateConfig, name, value)
    try:
        yield
    finally:
        for name, value in saved.items():
            setattr(ReplicateConfig, name, value)


@contextmanager
def with_custom_prompts(custom: Object) -> Generator[None]:
    with patch.object(litellm, "custom_prompt_dict", new=custom):
        yield


def standin_counter(**params: Json) -> int:
    # the count includes the model name's length, so a wrong model shows in the usage
    model = params.get("model")
    base = len(model) if isinstance(model, str) else 0
    messages = params.get("messages")
    if isinstance(messages, list):
        return base + len(messages)
    text = params.get("text")
    if isinstance(text, str):
        return base + len(text)
    return base


class SleepRecorder:
    def __init__(self) -> None:
        self.delays: list[float] = []

    def __call__(self, seconds: float) -> None:
        self.delays.append(seconds)


class AsyncSleepRecorder:
    def __init__(self) -> None:
        self.delays: list[float] = []

    async def __call__(self, seconds: float) -> None:
        self.delays.append(seconds)


class FakeResponse:
    def __init__(self, entry: Object) -> None:
        self.status_code = optional_int(entry.get("status"), 200)
        self.body = expect_text(entry["body"])
        self.headers = headers_value(entry.get("headers"))
        self.text = self.body

    def json(self) -> Json:
        return cast("Json", json.loads(self.body))


def request_output(
    method: str, url: str, headers: dict[str, str], data: str | None
) -> Object:
    return {
        "method": method,
        "url": url,
        "headers": {name.lower(): value for name, value in headers.items()},
        "body": None if data is None else cast("Json", json.loads(data)),
    }


class FakeClient:
    def __init__(self, create: FakeResponse, polls: list[FakeResponse]) -> None:
        self.create = create
        self.polls = list(polls)
        self.requests: list[Object] = []

    def post(self, url: str, headers: dict[str, str] | None = None, data: str = "") -> FakeResponse:
        self.requests.append(request_output("POST", url, headers or {}, data))
        return self.create

    def get(self, url: str, headers: dict[str, str] | None = None) -> FakeResponse:
        self.requests.append(request_output("GET", url, headers or {}, None))
        if not self.polls:
            raise RuntimeError("the poll script is exhausted")
        return self.polls.pop(0)

    def output(self) -> Json:
        return cast("Json", self.requests)


class FakePollClient:
    def __init__(self, polls: list[FakeResponse]) -> None:
        self.polls = list(polls)
        self.requests: list[Object] = []

    def get(self, url: str, headers: dict[str, str] | None = None) -> FakeResponse:
        self.requests.append(request_output("GET", url, headers or {}, None))
        if not self.polls:
            raise RuntimeError("the poll script is exhausted")
        return self.polls.pop(0)

    def output(self) -> Json:
        return cast("Json", self.requests)


class AsyncFakePollClient:
    def __init__(self, polls: list[FakeResponse]) -> None:
        self.polls = list(polls)
        self.requests: list[Object] = []

    async def get(self, url: str, headers: dict[str, str] | None = None) -> FakeResponse:
        self.requests.append(request_output("GET", url, headers or {}, None))
        if not self.polls:
            raise RuntimeError("the poll script is exhausted")
        return self.polls.pop(0)

    def output(self) -> Json:
        return cast("Json", self.requests)


def ignore_verbose(*_args: Json, **_kwargs: Json) -> None:
    pass


def describe_reply(reply: Completion) -> Object:
    usage = reply.usage
    return {
        "ok": True,
        "model": reply.model,
        "text": reply.choices[0].message.content,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    }


def map_params(input_value: Object) -> Json:
    cases = [expect_object(entry) for entry in expect_list(input_value["cases"])]
    results: list[Json] = []
    for entry in cases:
        options = dict(expect_object(entry.get("optional_params") or {}))
        MAP_PARAMS(
            non_default_params=dict(expect_object(entry.get("non_default") or {})),
            optional_params=options,
            model=expect_text(entry["model"]),
            drop_params=False,
        )
        results.append({"options": cast("Json", options)})
    return {
        "cases": results,
        "supported": list(SUPPORTED(model="model")),
    }


def urls(input_value: Object) -> Json:
    models = [expect_text(model) for model in expect_list(input_value["models"])]
    return {
        "urls": [
            COMPLETE_URL(
                api_base=None,
                api_key=None,
                model=model,
                optional_params={},
                litellm_params={},
            )
            for model in models
        ]
    }


def headers(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    found: list[Json] = []
    for entry in requests:
        merged = VALIDATE(
            headers_value(entry.get("headers")),
            "model",
            [],
            {},
            {},
            api_key=optional_text(entry.get("api_key")),
        )
        found.append(
            {
                expect_text(name).lower(): expect_text(value)
                for name, value in expect_object(merged).items()
            }
        )
    return {"headers": found}


def request(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    results: list[Json] = []
    for entry in requests:
        messages = expect_list(entry.get("messages") or [])
        options = dict(expect_object(entry.get("optional_params") or {}))
        defaults = expect_object(entry.get("defaults") or {})
        custom = expect_object(entry.get("custom_prompt_dict") or {})
        with with_defaults(defaults), with_custom_prompts(custom):
            try:
                body = TRANSFORM_REQUEST(
                    model=expect_text(entry["model"]),
                    messages=cast("list[dict[str, Json]]", messages),
                    optional_params=options,
                    litellm_params={},
                    headers={},
                )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                results.append(failure_from(error))
                continue
        results.append(
            {
                "ok": True,
                "body": cast("Json", body),
                "messages": messages,
                "options": cast("Json", options),
            }
        )
    return {"requests": results}


def reply(input_value: Object) -> Json:
    replies = [expect_object(entry) for entry in expect_list(input_value["replies"])]
    found: list[Json] = []
    for entry in replies:
        messages = expect_list(entry.get("messages") or [])
        with patch.object(transformation, "token_counter", standin_counter):
            try:
                result = TRANSFORM_RESPONSE(
                    model=expect_text(entry["model"]),
                    raw_response=FakeResponse(entry),
                    model_response=ModelResponse(),
                    logging_obj=Logging(),
                    request_data={"input": {}},
                    messages=cast("list[dict[str, Json]]", messages),
                    optional_params={},
                    litellm_params={},
                    encoding=None,
                )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                found.append(failure_from(error))
                continue
        found.append(describe_reply(cast("Completion", result)))
    return {"replies": found}


def poll(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    results: list[Json] = []
    for entry in requests:
        create = FakeResponse(expect_object(entry["create"]))
        polls = [
            FakeResponse(expect_object(item))
            for item in expect_list(entry.get("polls") or [])
        ]
        client = FakeClient(create, polls)
        sleep_recorder = SleepRecorder()
        messages = expect_list(entry.get("messages") or [])
        options = dict(expect_object(entry.get("optional_params") or {}))
        defaults = expect_object(entry.get("defaults") or {})
        custom = expect_object(entry.get("custom_prompt_dict") or {})
        retries = optional_int(entry.get("retries"), DEFAULT_RETRIES)
        retry_delay = optional_float(entry.get("retry_delay"), DEFAULT_RETRY_DELAY)
        now = optional_float(entry.get("now"), FIXED_NOW)
        with (
            patch.object(handler, "_get_httpx_client", return_value=client),
            patch.object(time, "sleep", new=sleep_recorder),
            patch.object(time, "time", return_value=now),
            patch.object(litellm, "DEFAULT_REPLICATE_POLLING_RETRIES", retries),
            patch.object(litellm, "DEFAULT_REPLICATE_POLLING_DELAY_SECONDS", retry_delay),
            patch.object(transformation, "token_counter", standin_counter),
            with_defaults(defaults),
            with_custom_prompts(custom),
        ):
            try:
                result = COMPLETION(
                    model=expect_text(entry["model"]),
                    messages=cast("list[dict[str, Json]]", messages),
                    api_base="https://ignored",
                    model_response=ModelResponse(),
                    print_verbose=ignore_verbose,
                    optional_params=options,
                    litellm_params={},
                    logging_obj=Logging(),
                    api_key="key",
                    encoding=None,
                )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                outcome = failure_from(error)
            else:
                completion = cast("Completion", result)
                outcome = {**describe_reply(completion), "created": completion.created}
        results.append(
            {
                "delays": cast("Json", sleep_recorder.delays),
                "requests": client.output(),
                **outcome,
            }
        )
    return {"requests": results}


def collect_sync(chunks: Iterator[Json]) -> tuple[list[Json], Json]:
    found: list[Json] = []
    try:
        found.extend(chunks)
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return found, failure_from(error)
    return found, None


async def collect_async(chunks: AsyncIterator[Json]) -> tuple[list[Json], Json]:
    found: list[Json] = []
    try:
        async for chunk in chunks:
            found.append(chunk)  # noqa: PERF401 - a failed stream keeps what it yielded
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return found, failure_from(error)
    return found, None


def stream(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    results: list[Json] = []
    for entry in requests:
        policy = expect_text(entry["policy"])
        url = expect_text(entry["url"])
        headers = headers_value(entry.get("headers"))
        polls = [
            FakeResponse(expect_object(item))
            for item in expect_list(entry.get("polls") or [])
        ]
        sync_sleep = SleepRecorder()
        async_sleep = AsyncSleepRecorder()
        polling_delay = optional_float(entry.get("polling_delay"), DEFAULT_POLLING_DELAY)
        with (
            patch.object(handler, "REPLICATE_POLLING_DELAY_SECONDS", polling_delay),
            patch.object(time, "sleep", new=sync_sleep),
            patch.object(asyncio, "sleep", new=async_sleep),
        ):
            if policy == "strict":
                client = FakePollClient(polls)
                chunks, error = collect_sync(
                    SYNC_STREAM(url, "token", ignore_verbose, headers, client)
                )
                requests_output = client.output()
            else:
                async_client = AsyncFakePollClient(polls)
                chunks, error = asyncio.run(
                    collect_async(ASYNC_STREAM(url, "token", ignore_verbose, headers, async_client))
                )
                requests_output = async_client.output()
        results.append(
            {
                "delays": cast("Json", sync_sleep.delays or async_sleep.delays),
                "chunks": cast("Json", chunks),
                "requests": requests_output,
                "error": error,
            }
        )
    return {"streams": results}


OPS: dict[str, Callable[[Object], Json]] = {
    "map_params": map_params,
    "urls": urls,
    "headers": headers,
    "request": request,
    "reply": reply,
    "poll": poll,
    "stream": stream,
}


def run(input_value: Object) -> Json:
    operation = expect_text(input_value["op"])
    return OPS[operation](input_value)
