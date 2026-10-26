import asyncio
from typing import Literal, TypeAlias, cast

import httpx
from litellm import DualCache
from litellm.proxy.guardrails.guardrail_hooks.lasso.lasso import LassoGuardrail, LassoResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def guardrail(settings: dict[str, str]) -> LassoGuardrail:
    return LassoGuardrail(
        api_key=settings.get("api_key"),
        api_base=settings.get("api_base"),
        user_id=settings.get("lasso_user_id"),
        conversation_id=settings.get("lasso_conversation_id"),
    )


async def call(lasso: LassoGuardrail, answer: Json, api_path: str | None) -> Json:
    paths: list[str] = []

    def handle(request: httpx.Request) -> httpx.Response:
        paths.append(request.url.path)
        match answer:
            case {"status": int(status), "body": body}:
                return httpx.Response(status, json=body)
            case _:
                raise httpx.ReadTimeout("the mock never answers", request=request)

    lasso.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
    api_url = f"{lasso.api_base}{api_path}" if api_path else None
    try:
        response = await lasso._call_lasso_api(headers={}, payload={}, api_url=api_url)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except Exception as error:
        await lasso._handle_api_error(error, "PROMPT")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        raise
    return {"path": paths[0], "response": cast(Json, response)}


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = cast(dict[str, Json], request)
    lasso = guardrail(cast(dict[str, str], fields["settings"]))
    data = cast(dict[str, Json], fields.get("data", {}))
    cache = DualCache()
    match operation:
        case "construct":
            settings = {"api_base": lasso.api_base, "user_id": lasso.user_id}
            return {**settings, "conversation_id": lasso.conversation_id}
        case "conversation_id":
            key = f"lasso_conversation_id:{data.get('litellm_call_id')}"
            if fields.get("cached") is not None:
                cache.set_cache(key, fields["cached"])  # pyright: ignore[reportUnknownMemberType]
            conversation_id = lasso._get_or_generate_conversation_id(data, cache)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            stored = cast(str | None, cache.get_cache(key))  # pyright: ignore[reportUnknownMemberType]
            return {"stored": stored == conversation_id, "id": conversation_id}
        case "headers":
            headers = lasso._prepare_headers(data, cache)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            return {name.lower(): value for name, value in headers.items()}
        case "payload":
            messages = cast(list[dict[str, object]], fields["messages"])
            message_type = cast(Literal["PROMPT", "COMPLETION"], fields["message_type"])
            payload = lasso._prepare_payload(messages, data, cache, message_type)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            return cast(Json, payload)
        case "process_response":
            lasso._process_lasso_response(cast(LassoResponse, fields["response"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            return {"allowed": True}
        case "call":
            api_path = cast(str | None, fields.get("api_path"))
            return asyncio.run(call(lasso, fields.get("answer"), api_path))
        case _:
            raise ValueError(f"unsupported operation {operation}")
