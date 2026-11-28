import asyncio
import contextlib
import io
import sys
from dataclasses import dataclass
from typing import Literal

from fastapi import Request
from litellm.proxy.lens import inference
from litellm.proxy.lens.models import Job, Lens, ModelRequest, Worker
from litellm.proxy.lens.repository import LensRepository
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

from .fixtures import (
    Call,
    Clock,
    StopEarlyError,
    Store,
    body_of,
    deployment_calls,
    deployments_of,
    job_of,
    lens_of,
    lens_state,
    model_info_of,
    prompt_of,
    readings_of,
    usage_of,
    worker_of,
)
from .wiring import (
    PROXY_MODULE,
    BillingModule,
    DatetimeModule,
    Key,
    LiteLLMModule,
    ProxyServer,
    Recorder,
    Router,
    reload_inference,
)


@dataclass(frozen=True)
class Subject:
    lens: Lens
    job: Job
    worker: Worker
    body: ModelRequest
    store: Store


def activate(call: Call, recorder: Recorder) -> None:
    billing = BillingModule(recorder, Key(call.get("team_id")))
    clock = DatetimeModule()
    reload_inference(LiteLLMModule(model_info_of(call), usage_of(call)), billing, clock)


def quote_value(call: Call) -> JsonValue:
    activate(call, Recorder("settle", None))
    return inference.quote(deployments_of(call), prompt_of(call))


def charge_value(call: Call) -> JsonValue:
    payload = call.get("response")
    estimate = call.get("estimate")
    if payload is None or estimate is None:
        raise ValueError("charge needs estimate and response")
    activate(call, Recorder("settle", estimate))
    return inference.completion_charge(deployments_of(call), ModelResponse(**payload), estimate)


def analyze_call(call: Call, subject: Subject) -> None:
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = ProxyServer(Router(deployment_calls(call)))
    scope: dict[str, JsonValue] = {"type": "http"}
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            asyncio.run(
                inference.analyze(
                    LensRepository(subject.store),
                    subject.lens,
                    subject.job,
                    subject.worker,
                    subject.body,
                    Request(scope),
                )
            )
    finally:
        if saved is None:
            sys.modules.pop(PROXY_MODULE)
        else:
            sys.modules[PROXY_MODULE] = saved


def analyze_value(call: Call, mode: Literal["reserve", "settle", "body"]) -> JsonValue:
    lens = lens_of(call)
    subject = Subject(
        lens=lens,
        job=job_of(call, lens),
        worker=worker_of(call),
        body=body_of(call),
        store=Store(lens),
    )
    recorder = Recorder(mode, call.get("cost"))
    Clock.readings = readings_of(call)
    Clock.position = 0
    activate(call, recorder)
    with contextlib.suppress(StopEarlyError):
        analyze_call(call, subject)
    if mode == "body":
        data = recorder.data
        if data is None:
            raise ValueError("the analysis body was not recorded")
        return TypeAdapter(JsonValue).validate_python(data)
    return lens_state(subject.store)


def run(input_value: Call) -> JsonValue:
    operation = input_value["operation"]
    if operation == "quote":
        return quote_value(input_value)
    if operation == "charge":
        return charge_value(input_value)
    if operation == "reserve":
        return analyze_value(input_value, "reserve")
    if operation == "settle":
        return analyze_value(input_value, "settle")
    return analyze_value(input_value, "body")
