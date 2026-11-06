import asyncio
from collections.abc import AsyncIterator
from typing import TypeAlias, cast

import httpx
from litellm import DualCache, ModelResponse
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.ibm_guardrails.ibm_detector import (
    IBMGuardrailDetector,
)
from litellm.types.guardrails import GuardrailEventHooks

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

GUARDRAIL_NAME = "ibm-detector"
TEST_BASE_URL = "https://ibm.test"
STREAM_AUTH = "stream"
STREAM_DETECTOR = "jailbreak-detector"
STANDARD_LOGGING_KEY = "standard_logging_guardrail_information"


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def mapping(value: Json | None) -> dict[str, Json]:
    return {} if value is None else object_of(value)


def boolean(value: Json | None, *, default: bool) -> bool:
    return default if value is None else cast(bool, value)


def guardrail(settings: dict[str, Json], *, default_on: bool) -> IBMGuardrailDetector:
    optional = mapping(settings.get("optional_params"))
    return IBMGuardrailDetector(
        guardrail_name=GUARDRAIL_NAME,
        auth_token=cast(str, settings["auth_token"]),
        base_url=cast(str, settings.get("base_url", TEST_BASE_URL)),
        detector_id=cast(str, settings["detector_id"]),
        is_detector_server=boolean(settings.get("is_detector_server"), default=True),
        detector_params=cast(dict[str, object], mapping(optional.get("detector_params"))),
        extra_headers=cast(dict[str, str], mapping(optional.get("extra_headers"))),
        score_threshold=cast(float | None, optional.get("score_threshold")),
        block_on_detection=boolean(optional.get("block_on_detection"), default=True),
        verify_ssl=boolean(settings.get("verify_ssl"), default=True),
        default_on=default_on,
        event_hook=cast(
            "GuardrailEventHooks | list[GuardrailEventHooks] | None", settings.get("mode")
        ),
    )


def install_mock(instance: IBMGuardrailDetector, answer: Json, captured: list[Json]) -> None:
    def handle(request: httpx.Request) -> httpx.Response:
        captured.append(
            {
                "path": request.url.path,
                "body": cast(Json, httpx.Response(200, content=request.content).json()),
            }
        )
        match answer:
            case {"status": int(status), "body": payload}:
                return httpx.Response(status, json=payload)
            case _:
                raise httpx.ReadTimeout("the mock never answers", request=request)

    instance.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))


def run_hook(kind: str, fields: dict[str, Json]) -> Json:
    instance = guardrail(mapping(fields.get("settings")), default_on=True)
    data = object_of(fields.get("data"))
    metadata_existed = "metadata" in data
    litellm_metadata_existed = "litellm_metadata" in data
    captured: list[Json] = []
    install_mock(instance, fields.get("answer"), captured)
    raised = False
    try:
        if kind == "pre_call":
            asyncio.run(
                instance.async_pre_call_hook(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                    UserAPIKeyAuth(), DualCache(), data, "acompletion"
                )
            )
        elif kind == "during_call":
            asyncio.run(
                instance.async_moderation_hook(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                    data=data, user_api_key_dict=UserAPIKeyAuth(), call_type="acompletion"
                )
            )
        else:
            response = ModelResponse(**cast(dict[str, object], object_of(fields["response"])))
            asyncio.run(
                instance.async_post_call_success_hook(  # pyright: ignore[reportUnknownMemberType]
                    data=data, user_api_key_dict=UserAPIKeyAuth(), response=response
                )
            )
    except Exception:  # noqa: BLE001 - any hook failure is one comparable outcome
        raised = True
    return {
        "requests": captured,
        "raised": raised,
        "metadata": visible_metadata(data, "metadata", existed=metadata_existed),
        "litellm_metadata": visible_metadata(
            data, "litellm_metadata", existed=litellm_metadata_existed
        ),
    }


def visible_metadata(data: dict[str, Json], key: str, *, existed: bool) -> Json:
    # the framework's standard logging record carries wall-clock timestamps and belongs to U1
    bucket = data.get(key)
    if not isinstance(bucket, dict):
        return bucket
    visible: dict[str, Json] = {
        name: value for name, value in bucket.items() if name != STANDARD_LOGGING_KEY
    }
    return visible if existed or visible else None


async def collect(stream: AsyncIterator[Json]) -> list[Json]:
    return [item async for item in stream]


def streaming(fields: dict[str, Json]) -> Json:
    items = cast(list[Json], fields["items"])
    instance = IBMGuardrailDetector(
        guardrail_name=GUARDRAIL_NAME,
        auth_token=STREAM_AUTH,
        base_url=TEST_BASE_URL,
        detector_id=STREAM_DETECTOR,
    )

    async def source() -> AsyncIterator[Json]:
        for item in items:
            yield item

    async def relay() -> list[Json]:
        stream = instance.async_post_call_streaming_iterator_hook(  # pyright: ignore[reportUnknownMemberType]
            UserAPIKeyAuth(), source(), {}
        )
        return await collect(cast(AsyncIterator[Json], stream))

    return {"items": asyncio.run(relay())}


def should_run(fields: dict[str, Json]) -> Json:
    instance = guardrail(mapping(fields.get("settings")), default_on=True)
    data: dict[str, object] = {}
    event_type = GuardrailEventHooks(cast(str, fields["event_type"]))
    return {
        "should_run": instance.should_run_guardrail(  # pyright: ignore[reportUnknownMemberType]
            data=data, event_type=event_type
        )
    }


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = object_of(request)
    match operation:
        case "pre_call":
            return run_hook("pre_call", fields)
        case "during_call":
            return run_hook("during_call", fields)
        case "post_call":
            return run_hook("post_call", fields)
        case "streaming":
            return streaming(fields)
        case "should_run":
            return should_run(fields)
        case _:
            raise ValueError(f"unsupported operation {operation}")
