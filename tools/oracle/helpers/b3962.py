import asyncio
import os
from collections.abc import Awaitable, Callable
from datetime import datetime
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.integrations.braintrust_logging import BraintrustLogger

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SUCCESS_STATUSES = range(200, 400)
ResponseObject: TypeAlias = (
    litellm.ModelResponse
    | litellm.TextCompletionResponse
    | litellm.EmbeddingResponse
    | litellm.ImageResponse
)


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def answer_list(scenario: dict[str, Json]) -> list[Json]:
    answers = scenario.get("answers")
    return list(answers) if isinstance(answers, list) else []


def apply_environment(scenario: dict[str, Json]) -> None:
    # every helper call is a fresh process, so the scenario replaces the inherited environment
    os.environ.clear()
    environment = scenario.get("environment")
    if not isinstance(environment, dict):
        return
    for name, value in environment.items():
        if isinstance(value, str):
            os.environ[name] = value


def optional_string(scenario: dict[str, Json], name: str) -> str | None:
    value = scenario.get(name)
    return value if isinstance(value, str) else None


class Recorder:
    def __init__(self, answers: list[Json]) -> None:
        self.requests: list[Json] = []
        self.answers = answers

    def record(self, url: str, headers: dict[str, Json] | None, body: Json) -> Json:
        normalized: dict[str, Json] = {
            name.lower(): value for name, value in (headers or {}).items()
        }
        self.requests.append({"url": str(url), "headers": normalized, "body": body})
        return self.answers.pop(0) if self.answers else {}


def reply(recorded: Json, url: str) -> httpx.Response:
    answer = recorded if isinstance(recorded, dict) else {}
    status = answer.get("status")
    status_code = status if isinstance(status, int) and not isinstance(status, bool) else 200
    response = httpx.Response(status_code, json=answer.get("body"))
    if status_code not in SUCCESS_STATUSES:
        raise httpx.HTTPStatusError(
            f"status {status_code}",
            request=httpx.Request("POST", url),
            response=response,
        )
    return response


class FakeSyncClient:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    def post(
        self,
        url: str,
        headers: dict[str, Json] | None = None,
        json: Json = None,
    ) -> httpx.Response:
        return reply(self.recorder.record(url, headers, json), url)


class FakeAsyncClient:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    async def post(
        self,
        url: str,
        headers: dict[str, Json] | None = None,
        json: Json = None,
    ) -> httpx.Response:
        return reply(self.recorder.record(url, headers, json), url)


def build_logger(scenario: dict[str, Json]) -> BraintrustLogger:
    apply_environment(scenario)
    return BraintrustLogger(
        api_key=optional_string(scenario, "api_key"),
        api_base=optional_string(scenario, "api_base"),
    )


def init_result(scenario: dict[str, Json]) -> Json:
    logger = build_logger(scenario)
    return cast(Json, {
        "api_base": logger.api_base,
        "api_key": logger.api_key,
    })


def resolve_sync(logger: BraintrustLogger, name: str) -> str:
    return logger.get_project_id_sync(name)


async def resolve_async(logger: BraintrustLogger, name: str) -> str:
    return await logger.get_project_id_async(name)


def project_result(scenario: dict[str, Json]) -> Json:
    logger = build_logger(scenario)
    recorder = Recorder(answer_list(scenario))
    name = string_value(scenario.get("project_name"), "project_name")
    calls = scenario.get("calls")
    count = calls if isinstance(calls, int) and not isinstance(calls, bool) else 1
    asynchronous = scenario.get("path") == "async"
    ids: list[Json] = []
    with (
        patch.object(logger, "global_braintrust_sync_http_handler", FakeSyncClient(recorder)),
        patch.object(logger, "global_braintrust_http_handler", FakeAsyncClient(recorder)),
    ):
        for _ in range(count):
            if asynchronous:
                ids.append(asyncio.run(resolve_async(logger, name)))
            else:
                ids.append(resolve_sync(logger, name))
    return cast(Json, {"requests": recorder.requests, "project_ids": ids})


def build_response(value: Json) -> ResponseObject | None:
    if not isinstance(value, dict):
        return None
    body = object_value(value.get("body"), "response.body")
    kind = value.get("kind")
    if kind == "model_response":
        return litellm.ModelResponse(**cast("dict[str, object]", body))
    if kind == "text_completion":
        return litellm.TextCompletionResponse(
            **cast("dict[str, object]", body)  # pyright: ignore[reportArgumentType]
        )
    if kind == "embedding":
        return litellm.EmbeddingResponse(
            **cast("dict[str, object]", body)  # pyright: ignore[reportArgumentType]
        )
    if kind == "image":
        return litellm.ImageResponse(
            **cast("dict[str, object]", body)  # pyright: ignore[reportArgumentType]
        )
    raise ValueError(f"unknown response kind: {kind}")


def parse_time(value: Json, name: str) -> datetime:
    return datetime.fromisoformat(string_value(value, name))


def build_kwargs(scenario: dict[str, Json]) -> dict[str, object]:
    metadata = scenario.get("metadata")
    kwargs: dict[str, object] = {
        "litellm_call_id": scenario.get("call_id"),
        "messages": scenario.get("messages"),
        "litellm_params": {"metadata": metadata if isinstance(metadata, dict) else {}},
        "response_cost": scenario.get("response_cost"),
        "call_type": scenario.get("call_type"),
        "api_call_start_time": (
            parse_time(scenario.get("api_call_start_time"), "api_call_start_time")
            if scenario.get("api_call_start_time") is not None
            else None
        ),
        "completion_start_time": (
            parse_time(scenario.get("completion_start_time"), "completion_start_time")
            if scenario.get("completion_start_time") is not None
            else None
        ),
    }
    standard = scenario.get("standard_logging_object")
    if isinstance(standard, dict):
        kwargs["standard_logging_object"] = standard
    return kwargs


def apply_default_tags(scenario: dict[str, Json]) -> None:
    tags = scenario.get("default_tags")
    if isinstance(tags, list):
        litellm.langfuse_default_tags = [tag for tag in tags if isinstance(tag, str)]
    else:
        litellm.langfuse_default_tags = None


def log_sync(
    logger: BraintrustLogger,
    kwargs: dict[str, object],
    response: ResponseObject | None,
    start: datetime,
    end: datetime,
) -> None:
    log_event = cast(Callable[..., None], logger.log_success_event)  # pyright: ignore[reportUnknownMemberType]
    log_event(kwargs, response, start, end)


async def log_async(
    logger: BraintrustLogger,
    kwargs: dict[str, object],
    response: ResponseObject | None,
    start: datetime,
    end: datetime,
) -> None:
    log_event = cast(Callable[..., Awaitable[None]], logger.async_log_success_event)  # pyright: ignore[reportUnknownMemberType]
    await log_event(kwargs, response, start, end)


def log_result(scenario: dict[str, Json]) -> Json:
    logger = build_logger(scenario)
    apply_default_tags(scenario)
    recorder = Recorder(answer_list(scenario))
    kwargs = build_kwargs(scenario)
    response = build_response(scenario.get("response"))
    start = parse_time(scenario.get("start_time"), "start_time")
    end = parse_time(scenario.get("end_time"), "end_time")
    calls = scenario.get("calls")
    count = calls if isinstance(calls, int) and not isinstance(calls, bool) else 1
    with (
        patch.object(logger, "global_braintrust_sync_http_handler", FakeSyncClient(recorder)),
        patch.object(logger, "global_braintrust_http_handler", FakeAsyncClient(recorder)),
    ):
        for _ in range(count):
            if scenario.get("path") == "async":
                asyncio.run(log_async(logger, kwargs, response, start, end))
            else:
                log_sync(logger, kwargs, response, start, end)
    return cast(Json, {"requests": recorder.requests})


HANDLERS = {
    "init": init_result,
    "resolve_project": project_result,
    "log": log_result,
}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    scenarios = input_value.get("scenarios")
    if not isinstance(scenarios, list):
        raise ValueError("scenarios must be a list")
    handler = HANDLERS.get(function) if isinstance(function, str) else None
    if handler is None:
        raise ValueError(f"unknown function: {function}")
    return {"results": [handler(object_value(value, "scenario")) for value in scenarios]}
