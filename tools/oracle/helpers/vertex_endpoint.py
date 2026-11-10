import logging
from collections.abc import Callable
from typing import Literal, TypeAlias, cast

import litellm
from litellm.llms.vertex_ai.common_utils import (
    construct_target_url,
    get_vertex_location_from_url,
    get_vertex_model_id_from_url,
    get_vertex_project_id_from_url,
)
from litellm.llms.vertex_ai.vertex_llm_base import VertexBase
from litellm.types.llms.vertex_ai import VertexPartnerProvider

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Mode: TypeAlias = Literal[
    "chat", "embedding", "batch_embedding", "image_generation", "count_tokens"
]
Version: TypeAlias = Literal["v1", "v1beta1"]
Provider: TypeAlias = Literal["vertex_ai", "vertex_ai_beta", "gemini"]
Handler: TypeAlias = Callable[[Params], Json]

PARTNERS: dict[str, VertexPartnerProvider] = {
    "llama": VertexPartnerProvider.llama,
    "mistralai": VertexPartnerProvider.mistralai,
    "ai21": VertexPartnerProvider.ai21,
    "claude": VertexPartnerProvider.claude,
}


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_field(params: Params, name: str) -> list[Json]:
    value = params.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(params: Params, name: str) -> str:
    value = params.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(params: Params, name: str) -> str | None:
    value = params.get(name)
    return value if isinstance(value, str) else None


def optional_bool(params: Params, name: str) -> bool | None:
    value = params.get(name)
    return value if isinstance(value, bool) else None


def bool_of(params: Params, name: str) -> bool:
    return optional_bool(params, name) is True


class WarningCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def region_row(item: Json) -> Json:
    fields = object_of(item, "region")
    model = string_of(fields, "model")
    configured = optional_string(fields, "region")
    logger = logging.getLogger("LiteLLM")
    capture = WarningCapture()
    logger.addHandler(capture)
    try:
        selected = VertexBase.get_vertex_region(configured, model)
    finally:
        logger.removeHandler(capture)
    return {
        "model": model,
        "configured": configured,
        "selected": selected,
        "warned": bool(capture.messages),
    }


def api_base_row(item: Json) -> Json:
    fields = object_of(item, "api_base")
    try:
        base = VertexBase().get_api_base(
            optional_string(fields, "api_base"), optional_string(fields, "location")
        )
    except ValueError:
        return {"error": True}
    return {"base": base}


def complete_row(item: Json) -> Json:
    fields = object_of(item, "complete")
    project_id = optional_string(fields, "project_id")
    try:
        url = VertexBase().get_complete_vertex_url(
            custom_api_base=optional_string(fields, "api_base"),
            vertex_location=optional_string(fields, "location"),
            vertex_project=optional_string(fields, "project"),
            project_id=project_id if project_id is not None else "",
            partner=PARTNERS[string_of(fields, "partner")],
            stream=optional_bool(fields, "stream"),
            model=string_of(fields, "model"),
        )
    except ValueError:
        return {"error": True}
    return {"url": url}


def create_row(item: Json) -> Json:
    fields = object_of(item, "create")
    try:
        url = VertexBase.create_vertex_url(
            vertex_location=string_of(fields, "location"),
            vertex_project=string_of(fields, "project"),
            partner=PARTNERS[string_of(fields, "partner")],
            stream=optional_bool(fields, "stream"),
            model=string_of(fields, "model"),
            api_base=optional_string(fields, "api_base"),
        )
    except ValueError:
        return {"error": True}
    return {"url": url}


def token_row(item: Json) -> Json:
    fields = object_of(item, "token")
    try:
        auth, url = VertexBase()._get_token_and_url(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            model=string_of(fields, "model"),
            auth_header=optional_string(fields, "auth"),
            gemini_api_key=optional_string(fields, "gemini_api_key"),
            vertex_project=optional_string(fields, "project"),
            vertex_location=optional_string(fields, "location"),
            vertex_credentials=None,
            stream=optional_bool(fields, "stream"),
            custom_llm_provider=cast(
                Provider, optional_string(fields, "provider") or "vertex_ai"
            ),
            api_base=optional_string(fields, "api_base"),
            should_use_v1beta1_features=bool_of(fields, "use_v1beta1_features"),
            mode=cast(Mode, optional_string(fields, "mode") or "chat"),
            use_psc_endpoint_format=bool_of(fields, "use_psc_endpoint_format"),
        )
    except ValueError:
        return {"error": True}
    return {"url": url, "auth": cast(Json, auth)}


def proxy_row(item: Json) -> Json:
    fields = object_of(item, "proxy")
    try:
        auth, url = VertexBase()._check_custom_proxy(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            api_base=optional_string(fields, "api_base"),
            custom_llm_provider=cast(
                Provider, optional_string(fields, "provider") or "vertex_ai"
            ),
            gemini_api_key=optional_string(fields, "gemini_api_key"),
            endpoint=string_of(fields, "endpoint"),
            stream=optional_bool(fields, "stream"),
            auth_header=optional_string(fields, "auth"),
            url=string_of(fields, "url"),
            model=optional_string(fields, "model"),
            vertex_project=optional_string(fields, "project"),
            vertex_location=optional_string(fields, "location"),
            vertex_api_version=cast(Version, optional_string(fields, "version")),
            use_psc_endpoint_format=bool_of(fields, "use_psc_endpoint_format"),
        )
    except ValueError:
        return {"error": True}
    return {"url": url, "auth": cast(Json, auth)}


def extras_of(fields: Params) -> dict[str, str]:
    value = fields.get("extras")
    if value is None:
        return {}
    extras: dict[str, str] = {}
    for name, text in object_of(value, "extras").items():
        if not isinstance(text, str):
            raise ValueError("extras values must be strings")
        extras[name] = text
    return extras


def header_row(item: Json) -> Json:
    fields = object_of(item, "header")
    headers = VertexBase().set_headers(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        optional_string(fields, "auth"), extras_of(fields)
    )
    return cast(Json, cast(dict[str, str], headers))


def route_operation(params: Params) -> Json:
    urls: list[Json] = []
    for item in list_field(params, "urls"):
        if not isinstance(item, str):
            raise ValueError("urls must be strings")
        urls.append(
            {
                "url": item,
                "project": get_vertex_project_id_from_url(item),
                "location": get_vertex_location_from_url(item),
                "model": get_vertex_model_id_from_url(item),
            }
        )
    targets: list[Json] = []
    for item in list_field(params, "routes"):
        fields = object_of(item, "route")
        try:
            url = construct_target_url(
                base_url=string_of(fields, "base_url"),
                requested_route=string_of(fields, "route"),
                vertex_location=optional_string(fields, "location"),
                vertex_project=optional_string(fields, "project"),
            )
        except ValueError:
            targets.append({"error": True})
        else:
            targets.append({"url": str(url)})
    return {"urls": urls, "targets": targets}


def target_rows(params: Params) -> Json:
    return {"targets": [proxy_row(item) for item in list_field(params, "targets")]}


HANDLERS: dict[str, Handler] = {
    "unsupported_region_warning": lambda params: {
        "regions": [region_row(item) for item in list_field(params, "regions")],
        "api_bases": [api_base_row(item) for item in list_field(params, "api_bases")],
    },
    "llama_openapi": lambda params: {
        "urls": [complete_row(item) for item in list_field(params, "complete")],
    },
    "ai21_stream": lambda params: {
        "complete": [complete_row(item) for item in list_field(params, "complete")],
        "create": [create_row(item) for item in list_field(params, "create")],
    },
    "host_and_version_grafts": lambda params: {
        "proxies": [proxy_row(item) for item in list_field(params, "proxies")],
        "tokens": [token_row(item) for item in list_field(params, "tokens")],
    },
    "psc_required_fields": target_rows,
    "gemini_custom_key": lambda params: {
        "tokens": [token_row(item) for item in list_field(params, "tokens")],
        "proxies": [proxy_row(item) for item in list_field(params, "proxies")],
    },
    "query_preserved": target_rows,
    "header_override": lambda params: {
        "headers": [header_row(item) for item in list_field(params, "headers")],
    },
    "cache_passthrough_path": route_operation,
}


def run(input_value: dict[str, Json]) -> Json:
    if len(input_value) != 1:
        raise ValueError("vertex_endpoint takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(object_of(payload, operation))
