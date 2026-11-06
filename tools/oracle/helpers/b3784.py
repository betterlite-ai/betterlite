import asyncio
import json
import os
from datetime import UTC, datetime, timezone, tzinfo
from types import SimpleNamespace
from typing import Literal, TypeAlias, cast
from unittest import mock

import httpx
from litellm.proxy.guardrails.guardrail_hooks.ovalix import initialize_guardrail
from litellm.proxy.guardrails.guardrail_hooks.ovalix import ovalix as ovalix_module
from litellm.proxy.guardrails.guardrail_hooks.ovalix.ovalix import OvalixGuardrail
from litellm.types.guardrails import GuardrailEventHooks, LitellmParams
from litellm.types.utils import GenericGuardrailAPIInputs

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class FrozenDatetime(datetime):
    fixed: datetime | None = None

    @classmethod
    def now(cls, tz: tzinfo | None = None) -> datetime:
        fixed = cls.fixed
        if fixed is None:
            raise RuntimeError("no frozen time")
        return fixed.astimezone(tz)


def frozen_clock(day: str) -> SimpleNamespace:
    year, month, date = (int(part) for part in day.split("-"))
    FrozenDatetime.fixed = datetime(year, month, date, 12, 0, tzinfo=UTC)
    return SimpleNamespace(datetime=FrozenDatetime, timezone=timezone)


def guardrail(fields: dict[str, Json]) -> OvalixGuardrail:
    settings = cast(dict[str, Json], fields.get("settings") or {})
    timeout_ms = cast(int | None, fields.get("timeout_ms"))
    parameters = LitellmParams.model_validate(
        {
            "guardrail": "ovalix",
            "mode": fields.get("mode", "pre_call"),
            "default_on": False,
            "timeout": None if timeout_ms is None else timeout_ms / 1000,
            **settings,
        }
    )
    return initialize_guardrail(
        parameters, {"guardrail_name": "ovalix", "litellm_params": parameters}
    )


def response_payload(instance: OvalixGuardrail, calls: list[tuple[str, Json]], texts: Json) -> Json:
    hooks = cast(list[GuardrailEventHooks], instance.supported_event_hooks)
    return {
        "tracker_api_base": cast(Json, instance._tracker_api_base),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "event_hooks": [hook.value for hook in hooks],
        "requests": [{"path": path, "body": body} for path, body in calls],
        "texts": texts,
    }


def ovalix_call(fields: dict[str, Json]) -> Json:
    for name, value in cast(dict[str, Json], fields.get("env") or {}).items():
        os.environ[name] = cast(str, value)
    instance = guardrail(fields)
    texts = cast(list[str], fields.get("texts") or [])
    answers = cast(list[dict[str, Json]], fields.get("answers") or [])
    request_data = cast(dict[str, Json], fields.get("request_data") or {})
    input_type = cast(Literal["request", "response"], fields.get("input_type", "request"))
    calls: list[tuple[str, Json]] = []
    pending = list(answers)

    def handle(request: httpx.Request) -> httpx.Response:
        calls.append((request.url.path, cast(Json, json.loads(request.content))))
        if not pending:
            raise httpx.ReadTimeout("the mock never answers", request=request)
        answer = pending.pop(0)
        return httpx.Response(cast(int, answer["status"]), json=answer.get("body"))

    async def call() -> GenericGuardrailAPIInputs:
        instance._async_handler.client = httpx.AsyncClient(  # noqa: SLF001  # pyright: ignore[reportAttributeAccessIssue, reportPrivateUsage]
            transport=httpx.MockTransport(handle)
        )
        inputs: GenericGuardrailAPIInputs = {"texts": texts}
        return await instance.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
            inputs=inputs,
            request_data=cast("dict[str, object]", request_data),  # pyright: ignore[reportArgumentType]
            input_type=input_type,
        )

    with mock.patch.object(ovalix_module, "datetime", frozen_clock(cast(str, fields["today"]))):
        result = asyncio.run(call())
    return response_payload(instance, calls, cast(Json, result.get("texts")))


def run(input_value: dict[str, Json]) -> Json:
    if "ovalix_call" in input_value:
        return ovalix_call(cast(dict[str, Json], input_value["ovalix_call"]))
    raise ValueError("unsupported operation")
