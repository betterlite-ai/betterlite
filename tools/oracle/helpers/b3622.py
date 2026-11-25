from typing import Literal, TypedDict

import litellm
from litellm.proxy.pass_through_endpoints import (
    llm_passthrough_endpoints as endpoint_module,
)
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from pydantic import JsonValue


class BaseUrlCall(TypedDict):
    function: Literal["base_url"]
    location: str | None


class RouterFields(TypedDict):
    litellm_params: dict[str, JsonValue] | None


class OverrideCall(TypedDict):
    function: Literal["override_params"]
    router_credentials: RouterFields | None
    project: str | None
    location: str | None


class HeadersCall(TypedDict):
    function: Literal["upstream_headers"]
    endpoint: str
    headers: dict[str, str]


Input = BaseUrlCall | OverrideCall | HeadersCall
Answer = str | list[str | None] | dict[str, str]


def override_answer(call: OverrideCall) -> list[str | None]:
    router = call["router_credentials"]
    if router is None:
        return [call["project"], call["location"]]
    managed = LiteLLM_ManagedVectorStore(litellm_params=router["litellm_params"])
    project, location = endpoint_module._override_vertex_params_from_router_credentials(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2400
        managed, call["project"], call["location"]
    )
    return [project, location]


def header_answer(call: HeadersCall) -> dict[str, str]:
    filtered = endpoint_module._upstream_headers_for_vertex_route(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2383
        call["endpoint"], call["headers"]
    )
    return dict(filtered)


def run(input_value: Input) -> Answer:
    litellm.suppress_debug_info = True
    match input_value["function"]:
        case "base_url":
            return endpoint_module.get_vertex_base_url(input_value["location"])
        case "override_params":
            return override_answer(input_value)
        case "upstream_headers":
            return header_answer(input_value)
