import asyncio
from types import SimpleNamespace
from typing import Literal, TypeAlias, cast

import httpx
from litellm.proxy.guardrails.guardrail_hooks.deepkeep import initialize_guardrail
from litellm.proxy.guardrails.guardrail_hooks.deepkeep.deepkeep import DeepKeepGuardrail
from litellm.types.guardrails import LitellmParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def guardrail(settings: dict[str, Json]) -> DeepKeepGuardrail:
    parameters = LitellmParams.model_validate(
        {"guardrail": "deepkeep", "mode": "pre_call", **settings}
    )
    return initialize_guardrail(
        parameters,
        {"guardrail_name": "firewall", "litellm_params": parameters},
    )


def construct(settings: dict[str, Json]) -> Json:
    deepkeep = guardrail(settings)
    timeout = deepkeep.timeout
    if timeout is None:
        timeout = deepkeep.async_handler.client.timeout
    seconds = timeout.read if isinstance(timeout, httpx.Timeout) else timeout
    return {
        "api_base": deepkeep.api_base,
        "timeout_ms": int(seconds * 1000) if seconds is not None else None,
        "supported_event_hooks": cast(Json, deepkeep.supported_event_hooks),
    }


def metadata(request_data: dict[str, Json]) -> Json:
    deepkeep = guardrail(
        {"api_key": "key", "api_base": "https://deepkeep.test", "deepkeep_firewall_id": "id"}
    )
    return cast(
        Json,
        deepkeep._extract_user_api_key_metadata(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]
            cast("dict[str, object]", request_data)  # pyright: ignore[reportArgumentType]
        ),
    )


async def apply(  # noqa: PLR0913, PLR0917
    deepkeep: DeepKeepGuardrail,
    answer: Json,
    inputs: dict[str, Json],
    request_data: dict[str, Json],
    dynamic_params: dict[str, Json],
    litellm_call_id: str | None,
    litellm_trace_id: str | None,
    input_type: str,
) -> Json:
    paths: list[str] = []
    bodies: list[Json] = []

    def handle(request: httpx.Request) -> httpx.Response:
        paths.append(request.url.path)
        bodies.append(cast(Json, httpx.Response(200, content=request.content).json()))
        match answer:
            case {"status": int(status), "body": body}:
                return httpx.Response(status, json=body)
            case _:
                raise httpx.ReadTimeout("the mock never answers", request=request)

    deepkeep.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
    deepkeep.get_guardrail_dynamic_request_body_params = (  # pyright: ignore[reportAttributeAccessIssue]
        lambda _request_body: dynamic_params  # pyright: ignore[reportUnknownLambdaType]
    )
    logging_obj = None
    if litellm_call_id is not None or litellm_trace_id is not None:
        logging_obj = SimpleNamespace(
            litellm_call_id=litellm_call_id, litellm_trace_id=litellm_trace_id
        )
    result = await deepkeep.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
        cast("Json", inputs),  # pyright: ignore[reportArgumentType]
        cast("dict[str, object]", request_data),
        cast(Literal["request", "response"], input_type),
        logging_obj,  # pyright: ignore[reportArgumentType]
    )
    return {
        "path": paths[0] if paths else None,
        "request": bodies[0] if bodies else None,
        "result": cast(Json, cast(object, result)),
    }


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = cast(dict[str, Json], request)
    match operation:
        case "construct":
            return construct(cast(dict[str, Json], fields["settings"]))
        case "metadata":
            return metadata(cast(dict[str, Json], fields["request_data"]))
        case "apply":
            deepkeep = guardrail(cast(dict[str, Json], fields["settings"]))
            return asyncio.run(
                apply(
                    deepkeep,
                    fields.get("answer"),
                    cast(dict[str, Json], fields["inputs"]),
                    cast(dict[str, Json], fields.get("request_data") or {}),
                    cast(dict[str, Json], fields.get("dynamic_params") or {}),
                    cast("str | None", fields.get("litellm_call_id")),
                    cast("str | None", fields.get("litellm_trace_id")),
                    cast(str, fields.get("input_type", "request")),
                )
            )
        case _:
            raise ValueError(f"unsupported operation {operation}")
