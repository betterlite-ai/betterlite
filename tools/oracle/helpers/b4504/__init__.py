import asyncio
import contextlib
import io
from collections.abc import Awaitable, Callable
from datetime import datetime, timezone, tzinfo
from typing import ClassVar, Literal, TypedDict
from unittest.mock import patch

import litellm
from fastapi import Request
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.lens import inference
from litellm.proxy.lens.models import Job, Lens, ModelRequest, Worker
from litellm.proxy.lens.repository import LensRepository
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

KEY = "a" * 64


class DeploymentParams(TypedDict, total=False):
    model: str
    input_cost_per_token: float | None
    output_cost_per_token: float | None


class DeploymentCall(TypedDict):
    litellm_params: DeploymentParams


class Call(TypedDict, total=False):
    operation: Literal["quote", "reserve", "settle", "charge", "request_body"]
    prompt: str
    deployments: list[DeploymentCall]
    model_info: dict[str, dict[str, JsonValue]]
    lens: dict[str, JsonValue]
    job_id: str
    worker_id: str
    now: list[str]
    team_id: str | None
    cost: float | None
    job: dict[str, JsonValue] | None
    estimate: float
    response: dict[str, JsonValue]


class Store:
    def __init__(self, lens: Lens, version: int) -> None:
        self.lens = lens
        self.version = version

    async def query_raw(self, query: str, *args: object) -> list[dict[str, object]]:
        if query.startswith("WITH previous"):
            if int(str(args[2])) != self.version:
                return [{"data": 0}]
            self.lens = Lens.model_validate_json(str(args[0]))
            self.version += 1
            return [{"data": 1}]
        if len(args) == 0 or str(args[0]) != self.lens.id:
            return []
        return [{"data": self.lens.model_dump(mode="json")}]

    async def execute_raw(self, query: str, *args: object) -> int:
        return 1


class Catalog:
    def __init__(self, entries: dict[str, dict[str, JsonValue]]) -> None:
        self.entries = entries

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> dict[str, JsonValue]:
        return self.entries[model]


class Router:
    def __init__(self, deployments: list[DeploymentCall]) -> None:
        self.deployments = deployments

    def get_model_list(self, model_name: str, team_id: str | None) -> list[DeploymentCall]:
        return self.deployments


class Key:
    def __init__(self, team_id: str | None) -> None:
        self.team_id = team_id

    async def __call__(self, key_id: str | None) -> UserAPIKeyAuth | None:
        return UserAPIKeyAuth(team_id=self.team_id)


class Clock(datetime):
    readings: ClassVar[tuple[datetime, ...]] = ()
    position: ClassVar[int] = 0

    @classmethod
    def now(cls, tz: tzinfo | None = None) -> "Clock":
        moment = Clock.readings[Clock.position]
        Clock.position += 1
        return cls(
            moment.year,
            moment.month,
            moment.day,
            moment.hour,
            moment.minute,
            moment.second,
            tzinfo=timezone.utc,
        )


class StopEarly(Exception):
    pass


class Recorder:
    def __init__(self, mode: str, cost: float | None) -> None:
        self.mode = mode
        self.cost = cost
        self.data: dict[str, object] | None = None

    async def __call__(
        self,
        key_id: str,
        data: dict[str, object],
        reserve: Callable[[], Awaitable[None]],
        incoming: Request,
    ) -> tuple[ModelResponse, float | None]:
        self.data = data
        if self.mode == "reserve":
            await reserve()
            raise StopEarly
        if self.mode == "body":
            raise StopEarly
        return (response(), self.cost)


def response() -> ModelResponse:
    return ModelResponse.model_validate(
        {
            "id": "x",
            "object": "chat.completion",
            "created": 1,
            "model": "lens-model",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "{}"}}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 4, "total_tokens": 6},
        }
    )


def deployments_of(call: Call) -> tuple[inference.Deployment, ...]:
    deployments = call.get("deployments")
    if deployments is None:
        raise ValueError("a call needs deployments")
    return tuple(inference.Deployment.model_validate(deployment) for deployment in deployments)


def model_info_of(call: Call) -> dict[str, dict[str, JsonValue]]:
    entries = call.get("model_info")
    return {} if entries is None else entries


def readings_of(call: Call) -> tuple[datetime, ...]:
    readings = call.get("now")
    if readings is None:
        raise ValueError("a call needs now")
    return tuple(datetime.fromisoformat(reading) for reading in readings)


def required(call: Call, key: str) -> str:
    value = call.get(key)
    if value is None:
        raise ValueError(f"a call needs {key}")
    return value


def lens_state(store: Store) -> JsonValue:
    lens = store.lens
    return {
        "version": store.version,
        "spent": lens.spent,
        "budget_month": lens.budget_month,
        "cost": lens.jobs[0].cost if lens.jobs else 0.0,
    }


def quote(call: Call) -> JsonValue:
    with patch.object(litellm, "get_model_info", Catalog(model_info_of(call))):
        return inference.quote(deployments_of(call), required(call, "prompt"))


def charge(call: Call) -> JsonValue:
    estimate = call.get("estimate")
    payload = call.get("response")
    if estimate is None or payload is None:
        raise ValueError("charge needs estimate and response")
    with patch.object(litellm, "get_model_info", Catalog(model_info_of(call))):
        return inference.completion_charge(
            deployments_of(call), ModelResponse.model_validate(payload), estimate
        )


def analyze(call: Call, mode: str) -> JsonValue:
    lens = Lens.model_validate(call.get("lens"))
    job_id = required(call, "job_id")
    payload = call.get("job")
    job = (
        Job.model_validate(payload)
        if payload is not None
        else next(job for job in lens.jobs if job.id == job_id)
    )
    worker = Worker.model_validate(
        {
            "id": required(call, "worker_id"),
            "name": "worker",
            "scope": {},
            "last_seen": "2026-01-01T00:00:00Z",
            "analysis_key_id": KEY,
        }
    )
    body = ModelRequest.model_validate({"prompt": required(call, "prompt"), "purpose": "cluster"})
    store = Store(lens, lens.version)
    recorder = Recorder(mode, call.get("cost"))
    Clock.readings = readings_of(call)
    Clock.position = 0
    with (
        patch("litellm.proxy.proxy_server.llm_router", Router(call.get("deployments", []))),
        patch("litellm.proxy.lens.inference.complete", recorder),
        patch("litellm.proxy.lens.inference.validate_key", Key(call.get("team_id"))),
        patch("litellm.proxy.lens.inference.datetime", Clock),
        patch.object(litellm, "get_model_info", Catalog(model_info_of(call))),
    ):
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                asyncio.run(
                    inference.analyze(
                        LensRepository(store), lens, job, worker, body, Request({"type": "http"})
                    )
                )
        except StopEarly:
            pass
    if mode == "body":
        data = recorder.data
        if data is None:
            raise ValueError("the analysis body was not recorded")
        return TypeAdapter(JsonValue).validate_python(data)
    return lens_state(store)


def run(input_value: Call) -> JsonValue:
    operation = input_value.get("operation")
    if operation == "quote":
        return quote(input_value)
    if operation == "charge":
        return charge(input_value)
    if operation == "reserve":
        return analyze(input_value, "reserve")
    if operation == "settle":
        return analyze(input_value, "settle")
    return analyze(input_value, "body")
