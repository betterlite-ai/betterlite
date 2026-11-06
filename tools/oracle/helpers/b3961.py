import json
import os
from collections.abc import Callable
from datetime import datetime, tzinfo
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.integrations.athina import AthinaLogger
from litellm.integrations.greenscale import GreenscaleLogger
from litellm.integrations.prompt_layer import PromptLayerLogger

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SUCCESS_STATUSES = range(200, 300)


class FrozenDatetime(datetime):
    fixed: datetime | None = None

    @classmethod
    def now(cls, tz: tzinfo | None = None) -> datetime:
        fixed = cls.fixed
        if fixed is None:
            raise RuntimeError("no frozen time")
        return fixed.astimezone(tz)


class FakeResponse:
    def __init__(self, status: int, body: Json) -> None:
        self.status_code = status
        self.text = json.dumps(body, default=str)
        self._body = body

    def json(self) -> Json:
        return self._body


def parse_body(data: Json) -> Json:
    if isinstance(data, str):
        return cast(Json, json.loads(data))
    return data


class FakeClient:
    def __init__(self, answers: list[Json]) -> None:
        self.requests: list[Json] = []
        self.answers = answers

    def post(
        self,
        url: str,
        headers: dict[str, Json] | None = None,
        data: Json = None,
        json: Json = None,
    ) -> FakeResponse:
        normalized: dict[str, Json] = {
            name.lower(): value for name, value in (headers or {}).items()
        }
        body: Json
        if json is not None:
            body = json
            normalized.setdefault("content-type", "application/json")
        else:
            body = parse_body(data)
        self.requests.append({"url": str(url), "headers": normalized, "body": body})
        answer = cast(dict[str, Json], self.answers.pop(0) if self.answers else {})
        failure = answer.get("raise")
        if failure is not None:
            raise RuntimeError(str(failure))
        status = answer.get("status")
        status_code = status if isinstance(status, int) and not isinstance(status, bool) else 200
        if status_code not in SUCCESS_STATUSES:
            raise RuntimeError(f"status {status_code}")
        response_body = answer.get("body", {})
        return FakeResponse(status_code, response_body)


def apply_environment(scenario: dict[str, Json]) -> None:
    # every helper call is a fresh process, so the scenario replaces the inherited environment
    os.environ.clear()
    environment = scenario.get("environment")
    if not isinstance(environment, dict):
        return
    for name, value in environment.items():
        if isinstance(value, str):
            os.environ[name] = value


def parse_time(value: Json, kind: type[datetime] = datetime) -> datetime:
    if not isinstance(value, str):
        raise ValueError("time must be a string")
    return kind.fromisoformat(value)


def model_response(value: Json | None) -> litellm.ModelResponse | None:
    if not isinstance(value, dict):
        return None
    return litellm.ModelResponse(**cast("dict[str, object]", value))


def answer_list(scenario: dict[str, Json]) -> list[Json]:
    answers = scenario.get("answers")
    return list(answers) if isinstance(answers, list) else []


def log_result(client: FakeClient, *, raised: bool) -> Json:
    return {"requests": client.requests, "raised": raised}


def run_log_event(client: FakeClient, call: Callable[[], None]) -> bool:
    with patch.object(litellm, "module_level_client", client):
        try:
            call()
        except Exception:  # noqa: BLE001 - the helper records whether the logger re-raised
            return True
    return False


def ignore_verbose(_message: str) -> None:
    return None


def invoke_logger(
    logger: AthinaLogger | GreenscaleLogger | PromptLayerLogger,
    kwargs: dict[str, object],
    response: litellm.ModelResponse | None,
    start: datetime,
    end: datetime,
) -> None:
    log_event = cast(Callable[..., None], logger.log_event)  # pyright: ignore[reportUnknownMemberType]
    log_event(kwargs, response, start, end, ignore_verbose)


def athina_init(scenario: dict[str, Json]) -> Json:
    apply_environment(scenario)
    logger = AthinaLogger()
    return cast(Json, {
        "headers": {name.lower(): value for name, value in logger.headers.items()},
        "url": logger.athina_logging_url,
        "additional_keys": logger.additional_keys,
    })


def plain_log(
    scenario: dict[str, Json],
    logger: type[AthinaLogger] | type[PromptLayerLogger],
) -> Json:
    apply_environment(scenario)
    kwargs: dict[str, object] = dict(cast("dict[str, object]", scenario.get("kwargs") or {}))
    if "complete_streaming_response" in scenario:
        kwargs["complete_streaming_response"] = model_response(
            scenario["complete_streaming_response"]
        )
    start = parse_time(scenario.get("start_time"))
    end = parse_time(scenario.get("end_time"))
    response = model_response(scenario.get("response"))
    client = FakeClient(answer_list(scenario))
    raised = run_log_event(
        client,
        lambda: invoke_logger(logger(), kwargs, response, start, end),
    )
    return log_result(client, raised=raised)


def greenscale_init(scenario: dict[str, Json]) -> Json:
    apply_environment(scenario)
    logger = GreenscaleLogger()
    return cast(Json, {
        "headers": {name.lower(): value for name, value in logger.headers.items()},
        "url": logger.greenscale_logging_url,
    })


def greenscale_log(scenario: dict[str, Json]) -> Json:
    apply_environment(scenario)
    kwargs: dict[str, object] = dict(cast("dict[str, object]", scenario.get("kwargs") or {}))
    logger = GreenscaleLogger()
    start = parse_time(scenario.get("start_time"), FrozenDatetime)
    end = parse_time(scenario.get("end_time"), FrozenDatetime)
    FrozenDatetime.fixed = parse_time(scenario.get("now"))
    response = model_response(scenario.get("response"))
    client = FakeClient(answer_list(scenario))
    with (
        patch.object(litellm, "module_level_client", client),
        patch("litellm.integrations.greenscale.datetime", FrozenDatetime),
    ):
        raised = run_log_event(
            client,
            lambda: invoke_logger(logger, kwargs, response, start, end),
        )
    return log_result(client, raised=raised)


def promptlayer_init(scenario: dict[str, Json]) -> Json:
    apply_environment(scenario)
    logger = PromptLayerLogger()
    return cast(Json, {"key": logger.key})


INIT_LOGGERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "athina": athina_init,
    "greenscale": greenscale_init,
    "promptlayer": promptlayer_init,
}


def log_result_for(scenario: dict[str, Json], name: Json) -> Json:
    if name == "greenscale":
        return greenscale_log(scenario)
    loggers: dict[str, type[AthinaLogger | PromptLayerLogger]] = {
        "athina": AthinaLogger,
        "promptlayer": PromptLayerLogger,
    }
    logger = loggers.get(name) if isinstance(name, str) else None
    if logger is None:
        raise ValueError(f"unknown logger: {name}")
    return plain_log(scenario, logger)


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    scenarios = input_value.get("scenarios")
    if function not in ("init", "log") or not isinstance(scenarios, list):
        raise ValueError(f"expected init or log with scenarios, got {function}")
    results: list[Json] = []
    for value in scenarios:
        scenario = cast(dict[str, Json], value)
        name = scenario.get("logger")
        if function == "init":
            handler = INIT_LOGGERS.get(name) if isinstance(name, str) else None
            if handler is None:
                raise ValueError(f"unknown logger: {name}")
            results.append(handler(scenario))
        else:
            results.append(log_result_for(scenario, name))
    return {"results": results}
