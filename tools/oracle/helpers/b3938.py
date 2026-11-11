import asyncio
import os
from collections.abc import Mapping
from datetime import UTC, datetime
from types import ModuleType
from typing import Literal, NotRequired, Required, TypedDict, cast

import httpx
import litellm
from litellm.integrations.datadog import datadog_llm_obs
from litellm.integrations.datadog.datadog_llm_obs import DataDogLLMObsLogger
from litellm.litellm_core_utils.dd_tracing import NullTracer
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.integrations.datadog_llm_obs import DatadogLLMObsInitParams
from pydantic import JsonValue, TypeAdapter

JsonObject = dict[str, JsonValue]
JSON_BODY = TypeAdapter(JsonValue)


class Reply(TypedDict):
    status: int
    body: NotRequired[str]


class Init(TypedDict, total=False):
    dd_api_key: str
    dd_site: str
    dd_agent_host: str
    dd_agent_port: str
    turn_off_message_logging: bool
    message_logging: bool
    batch_size: int
    flush_interval: float
    global_turn_off: bool


class Params(TypedDict, total=False):
    turn_off_message_logging: bool


class Action(TypedDict):
    do: Literal["success", "failure", "flush"]
    payload: NotRequired[JsonObject]
    metadata: NotRequired[JsonObject]
    start_time: NotRequired[float]
    end_time: NotRequired[float]


class Scenario(TypedDict, total=False):
    actions: Required[list[Action]]
    name: str
    environment: dict[str, str]
    init: Init
    params: Params
    apm_trace_id: int
    replies: list[Reply]


class Input(TypedDict):
    scenarios: list[Scenario]


class _Span:
    def __init__(self, trace_id: int) -> None:
        self.trace_id = trace_id


class _Uuids(ModuleType):
    @staticmethod
    def uuid4() -> str:
        return "generated-id"


class _Tracer(NullTracer):
    def __init__(self, trace_id: int | None) -> None:
        self.trace_id = trace_id

    def current_span(self) -> _Span | None:
        if self.trace_id is None:
            return None
        return _Span(self.trace_id)


class _Client:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.calls: list[JsonValue] = []

    async def post(self, url: str, content: str, headers: Mapping[str, str]) -> httpx.Response:
        self.calls.append(
            {
                "url": url,
                "headers": {name.lower(): value for name, value in headers.items()},
                "body": JSON_BODY.validate_json(content),
            }
        )
        reply = self.replies.pop(0)
        body = reply.get("body")
        request = httpx.Request("POST", url)
        return httpx.Response(
            reply["status"], text="" if body is None else body, request=request
        )


def _apply_environment(environment: Mapping[str, str]) -> None:
    for name in list(os.environ):
        if name.startswith(("DD_", "LITELLM_DD_")):
            del os.environ[name]
    for name, value in environment.items():
        os.environ[name] = value


def _init_params(params: Params | None) -> DatadogLLMObsInitParams | None:
    if params is None:
        return None
    flag = params.get("turn_off_message_logging")
    if flag is None:
        return None
    return DatadogLLMObsInitParams(turn_off_message_logging=flag)


def _logger_kwargs(init: Init) -> JsonObject:
    kwargs: JsonObject = {}
    turn_off = init.get("turn_off_message_logging")
    if turn_off is not None:
        kwargs["turn_off_message_logging"] = turn_off
    message_logging = init.get("message_logging")
    if message_logging is not None:
        kwargs["message_logging"] = message_logging
    batch_size = init.get("batch_size")
    if batch_size is not None:
        kwargs["batch_size"] = batch_size
    flush_interval = init.get("flush_interval")
    if flush_interval is not None:
        kwargs["flush_interval"] = flush_interval
    return kwargs


def _build_logger(init: Init, environment: Mapping[str, str]) -> DataDogLLMObsLogger:
    _apply_environment(environment)
    global_turn_off = init.get("global_turn_off")
    litellm.turn_off_message_logging = False if global_turn_off is None else global_turn_off
    logger = DataDogLLMObsLogger(**_logger_kwargs(init))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L441
    batch_size = init.get("batch_size")
    if batch_size is None and environment.get("DEFAULT_BATCH_SIZE"):
        logger.batch_size = int(environment["DEFAULT_BATCH_SIZE"])
    return logger


def _queue(logger: DataDogLLMObsLogger) -> list[JsonValue]:
    # the base logger types its queue as a plain list
    return cast("list[JsonValue]", logger.log_queue)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_batch_logger.py#L39


async def _perform(logger: DataDogLLMObsLogger, action: Action) -> None:
    step = action["do"]
    if step == "flush":
        await logger.flush_queue()  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_batch_logger.py#L54
        return
    metadata = action.get("metadata")
    kwargs: JsonObject = {
        "standard_logging_object": action["payload"],
        "litellm_params": {"metadata": {} if metadata is None else metadata},
    }
    start = datetime.fromtimestamp(action["start_time"], tz=UTC)
    end = datetime.fromtimestamp(action["end_time"], tz=UTC)
    if step == "success":
        await logger.async_log_success_event(kwargs, None, start, end)  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L531
    else:
        await logger.async_log_failure_event(kwargs, None, start, end)  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L543


def _init_report(logger: DataDogLLMObsLogger) -> JsonValue:
    return {
        "batch_size": logger.batch_size,
        "intake_url": logger.intake_url,
        "authenticated": logger.DD_API_KEY is not None,
        "error": None,
    }


async def _execute(scenario: Scenario, client: _Client) -> JsonValue:
    init = scenario.get("init")
    environment = scenario.get("environment")
    apm_trace_id = scenario.get("apm_trace_id")
    datadog_llm_obs.tracer = _Tracer(apm_trace_id)  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L34
    datadog_llm_obs.uuid = _Uuids("uuid")  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L666
    litellm.datadog_llm_observability_params = _init_params(scenario.get("params"))
    try:
        logger = _build_logger(
            {} if init is None else init, {} if environment is None else environment
        )
    except Exception as error:  # noqa: BLE001 - the helper reports which setting was missing  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L462
        reason = "missing_api_key" if "DD_API_KEY" in str(error) else "missing_site"
        return {"name": scenario.get("name"), "init": {"error": reason}}
    logger.async_client = cast(AsyncHTTPHandler, client)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L456
    results: list[JsonValue] = []
    for action in scenario["actions"]:
        await _perform(logger, action)
        results.append({"queue_length": len(_queue(logger))})
    queue = _queue(logger)
    return {
        "name": scenario.get("name"),
        "init": _init_report(logger),
        "results": results,
        "calls": client.calls,
        "queue": {"length": len(queue), "head": queue[:3], "tail": queue[-3:]},
    }


async def _run_all(input_value: Input) -> JsonValue:
    scenarios: list[JsonValue] = []
    for scenario in input_value["scenarios"]:
        replies = scenario.get("replies")
        client = _Client([] if replies is None else replies)
        scenarios.append(await _execute(scenario, client))
    return {"scenarios": scenarios}


def run(input_value: Input) -> JsonValue:
    if not input_value["scenarios"]:
        return {"scenarios": []}
    return asyncio.run(_run_all(input_value))
