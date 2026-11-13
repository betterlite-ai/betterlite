import asyncio
import os
from typing import Literal, TypeAlias, cast

import httpx
import litellm
from fastapi import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.openai.moderations import (
    OpenAIModerationGuardrail,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

RECORD_KEYS = ("guardrail_name", "guardrail_response", "guardrail_status", "violation_categories")


def guardrail(
    settings: dict[str, Json], environment: dict[str, str], global_keys: dict[str, Json]
) -> OpenAIModerationGuardrail:
    os.environ.pop("OPENAI_API_KEY", None)
    os.environ.update(environment)
    litellm.api_key = cast("str | None", global_keys.get("api_key"))
    litellm.openai_key = cast("str | None", global_keys.get("openai_key"))
    return OpenAIModerationGuardrail(
        guardrail_name="moderation",
        **cast("dict[str, object]", settings),  # pyright: ignore[reportArgumentType]
    )


def construct(
    settings: dict[str, Json], environment: dict[str, str], global_keys: dict[str, Json]
) -> Json:
    moderation = guardrail(settings, environment, global_keys)
    return {
        "api_key": moderation.api_key,
        "api_base": moderation.api_base,
        "model": moderation.model,
        "streaming_end_of_stream_only": moderation.streaming_end_of_stream_only,
        "streaming_sampling_rate": moderation.streaming_sampling_rate,
        "supported_event_hooks": [hook.value for hook in moderation.supported_event_hooks or []],
    }


def record(entry: dict[str, Json]) -> dict[str, Json]:
    kept = {key: entry[key] for key in RECORD_KEYS if key in entry}
    response = kept["guardrail_response"]
    if isinstance(response, str) and response != "allow":
        kept["guardrail_response"] = "<error>"
    return kept


async def apply(
    moderation: OpenAIModerationGuardrail,
    answer: Json,
    inputs: dict[str, Json],
    request_data: dict[str, Json],
) -> Json:
    seen: list[httpx.Request] = []

    def handle(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        match answer:
            case {"status": int(status), "body": body}:
                return httpx.Response(status, json=body)
            case _:
                raise httpx.ReadTimeout("the mock never answers", request=request)

    moderation.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
    error: Json = None
    result: Json = None
    try:
        result = cast(
            Json,
            await moderation.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
                inputs=cast("dict[str, object]", inputs),  # pyright: ignore[reportArgumentType]
                request_data=cast("dict[str, object]", request_data),
                input_type=cast(Literal["request", "response"], "request"),
            ),
        )
    except HTTPException as exc:
        error = {"status": exc.status_code, "detail": cast(Json, exc.detail)}
    except httpx.HTTPStatusError as exc:
        # LiteLLM's HTTP handler masks every 4xx and 5xx before the guardrail sees it.
        error = {"status": exc.response.status_code, "detail": None}
    sent = None
    if seen:
        request = seen[0]
        sent = {
            "path": request.url.path.rsplit("/", 1)[-1],
            "body": cast(Json, httpx.Response(200, content=request.content).json()),
            "authorization_ok": request.headers["authorization"] == f"Bearer {moderation.api_key}",
            "content_type": request.headers["content-type"],
        }
    metadata = cast(dict[str, Json], request_data.get("metadata") or {})
    records = cast(
        "list[dict[str, Json]]", metadata.get("standard_logging_guardrail_information") or []
    )
    return {
        "request": sent,
        "inputs_unchanged": result == inputs,
        "error": error,
        "stash_left": "_openai_moderation_response" in metadata,
        "records": [record(entry) for entry in records],
    }


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = cast(dict[str, Json], request)
    settings = cast(dict[str, Json], fields.get("settings") or {})
    environment = cast(dict[str, str], fields.get("environment") or {})
    global_keys = cast(dict[str, Json], fields.get("global_keys") or {})
    match operation:
        case "construct":
            return construct(settings, environment, global_keys)
        case "apply":
            moderation = guardrail(settings, environment, global_keys)
            return asyncio.run(
                apply(
                    moderation,
                    fields.get("answer"),
                    cast(dict[str, Json], fields["inputs"]),
                    cast(dict[str, Json], fields.get("request_data") or {}),
                )
            )
        case _:
            raise ValueError(f"unsupported operation {operation}")
