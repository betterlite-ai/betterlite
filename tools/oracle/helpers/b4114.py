from collections.abc import Mapping
from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.integrations.websearch_interception.handler import WebSearchInterceptionLogger

if TYPE_CHECKING:
    from litellm.proxy._types import UserAPIKeyAuth

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    if operation == "select_tool":
        return select_tool(input_value)
    if operation == "caller_from_kwargs":
        return caller_from_kwargs(input_value)
    if operation == "parent_correlation":
        return parent_correlation(input_value)
    raise ValueError(f"unknown b4114 operation: {operation!r}")


def select_tool(input_value: dict[str, Json]) -> Json:
    name_value = input_value.get("search_tool_name")
    search_tool_name = name_value if isinstance(name_value, str) else None
    logger = WebSearchInterceptionLogger(search_tool_name=search_tool_name)
    tools_value = input_value.get("search_tools")
    if tools_value is None:
        router: object = None
    elif isinstance(tools_value, list):
        router = SimpleNamespace(search_tools=cast("list[Json]", tools_value))
    else:
        raise ValueError("search_tools must be a list or null")
    selected = logger._select_search_tool_from_router(router)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return tool_value(cast("Mapping[str, object] | None", selected))


def tool_value(tool: Mapping[str, object] | None) -> Json:
    if tool is None:
        return None
    params_value = tool.get("litellm_params")
    params: Mapping[str, object] = (
        cast("Mapping[str, object]", params_value) if isinstance(params_value, dict) else {}
    )
    return {
        "search_tool_name": cast(Json, tool.get("search_tool_name")),
        "search_provider": cast(Json, params.get("search_provider")),
        "litellm_params": {
            key: cast(Json, value) for key, value in params.items() if key != "search_provider"
        },
    }


def caller_from_kwargs(input_value: dict[str, Json]) -> Json:
    kwargs = build_kwargs(input_value.get("kwargs"))
    found = WebSearchInterceptionLogger._get_user_api_key_auth_from_kwargs(kwargs)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if found is None:
        return None
    return {
        "team_id": found.team_id,
        "parent_otel_span": cast(Json, found.parent_otel_span),
    }


def parent_correlation(input_value: dict[str, Json]) -> Json:
    kwargs = build_kwargs(input_value.get("kwargs"))
    caller = cast("UserAPIKeyAuth | None", caller_object(input_value.get("caller")))
    correlation = WebSearchInterceptionLogger._get_parent_request_correlation(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        kwargs,
        caller,
    )
    return {
        "correlation": {
            "session_id": correlation.session_id,
            "trace_id": correlation.trace_id,
            "parent_request_id": correlation.parent_request_id,
            "parent_otel_span": cast(Json, correlation.parent_otel_span),
        },
        "metadata": cast(Json, dict(correlation.as_search_metadata())),
        "kwargs": cast(Json, dict(correlation.as_search_kwargs())),
    }


def build_kwargs(value: Json | None) -> dict[str, object] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("kwargs must be an object or null")
    fields = cast("dict[object, object]", value)
    kwargs: dict[str, object] = {str(key): item for key, item in fields.items()}
    for key in ("metadata", "litellm_metadata"):
        if key in kwargs:
            kwargs[key] = metadata_block(kwargs[key])
    params = kwargs.get("litellm_params")
    if isinstance(params, dict):
        scoped_fields = cast("dict[object, object]", params)
        scoped: dict[str, object] = {str(key): item for key, item in scoped_fields.items()}
        for key in ("metadata", "litellm_metadata"):
            if key in scoped:
                scoped[key] = metadata_block(scoped[key])
        kwargs["litellm_params"] = scoped
    return kwargs


def metadata_block(value: object) -> object:
    if not isinstance(value, dict):
        return value
    fields = cast("dict[object, object]", value)
    block: dict[str, object] = {str(key): item for key, item in fields.items()}
    block["user_api_key_auth"] = caller_object(block.get("user_api_key_auth"))
    return block


def caller_object(value: object) -> object:
    if value is None:
        return None
    if not isinstance(value, dict):
        return value
    fields = cast("dict[object, object]", value)
    return SimpleNamespace(
        team_id=fields.get("team_id"),
        parent_otel_span=fields.get("parent_otel_span"),
    )
