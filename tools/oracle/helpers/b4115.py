import asyncio
from typing import Literal, Required, TypeAlias, TypedDict, cast

import litellm
from litellm.exceptions import BadRequestError, RateLimitError
from litellm.integrations.websearch_interception.handler import WebSearchInterceptionLogger
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.proxy import proxy_server
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.router import Router
from litellm.types.integrations.websearch_interception import RichWebSearchInput
from litellm.types.utils import LlmProviders

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Tool(TypedDict, total=False):
    type: str
    name: str


class SearchError(TypedDict):
    kind: Literal["rate_limit", "bad_request", "unavailable"]
    message: str


class SearchTool(TypedDict, total=False):
    search_tool_name: str
    litellm_params: dict[str, Json]


class Input(TypedDict, total=False):
    operation: Required[
        Literal[
            "short_circuit",
            "execute_search",
            "rich_search_input",
            "provider_supports_rich_search",
        ]
    ]
    model: Required[str]
    messages: list[dict[str, Json]]
    tools: list[dict[str, Json]]
    custom_llm_provider: str | None
    enabled_providers: list[str]
    search_tools: list[SearchTool]
    search_tool_name: str
    search_response: dict[str, Json]
    search_error: SearchError
    query: Required[str]
    tool_inputs: list[Json]
    search_providers: list[str | None]
    caller: bool


class ScriptedRouter:
    def __init__(self, search_tools: list[SearchTool]) -> None:
        self.search_tools = search_tools


RECORDED: list[Json] = []
SCRIPTED_RESPONSE: list[SearchResponse] = []
SCRIPTED_ERROR: list[BaseException] = []


async def fake_asearch(
    query: str | list[Json], search_provider: str, **kwargs: Json
) -> SearchResponse:
    params = dict(kwargs)
    metadata = params.pop("litellm_metadata", None)
    call: dict[str, Json] = {
        "query": query,
        "search_provider": search_provider,
        "params": params,
        "metadata": metadata,
    }
    RECORDED.append(call)
    if SCRIPTED_ERROR:
        raise SCRIPTED_ERROR[0]
    if SCRIPTED_RESPONSE:
        return SCRIPTED_RESPONSE[0]
    raise ValueError("the case scripts no search response")


async def deny_key(search_tool_name: str, valid_token: UserAPIKeyAuth) -> Literal[True]:  # noqa: ARG001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5351-L5374
    raise ProxyException(
        message=f"key may not call the search tool {search_tool_name}",
        type="auth_error",
        param=None,
        code=401,
    )


def scripted_error(spec: SearchError) -> BaseException:
    if spec["kind"] == "rate_limit":
        return RateLimitError.__new__(RateLimitError)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L442-L520
    if spec["kind"] == "bad_request":
        return BadRequestError.__new__(BadRequestError)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219-L280
    return ValueError(spec["message"])


def install(input_value: Input) -> None:
    RECORDED.clear()
    SCRIPTED_RESPONSE.clear()
    SCRIPTED_ERROR.clear()
    response = input_value.get("search_response")
    if response is not None:
        SCRIPTED_RESPONSE.append(SearchResponse.model_validate(response))
    error = input_value.get("search_error")
    if error is not None:
        SCRIPTED_ERROR.append(scripted_error(error))
    litellm.asearch = fake_asearch  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/search/main.py#L59-L159
    tools = input_value.get("search_tools")
    if tools is not None:
        proxy_server.llm_router = cast("Router", ScriptedRouter(tools))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2584
    if input_value.get("caller"):
        auth_checks.can_key_call_search_tool = deny_key  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5351-L5374


def logger_for(input_value: Input) -> WebSearchInterceptionLogger:
    enabled: list[LlmProviders | str] = []
    providers = input_value.get("enabled_providers")
    if providers is not None:
        enabled = list(providers)
    return WebSearchInterceptionLogger(
        enabled_providers=enabled,
        search_tool_name=input_value.get("search_tool_name"),
    )


def kwargs_for(input_value: Input) -> dict[str, Json] | None:
    if not input_value.get("caller"):
        return None
    caller: dict[str, Json] = {"team_id": None}
    metadata: dict[str, Json] = {"user_api_key_auth": caller}
    kwargs: dict[str, Json] = {"metadata": metadata}
    return kwargs


def rich_value(input_value: Input) -> RichWebSearchInput | None:
    tool_inputs = input_value.get("tool_inputs")
    if not tool_inputs:
        return None
    return WebSearchInterceptionLogger._rich_search_input(tool_inputs[0])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1481-L1507


async def short_circuit(input_value: Input) -> Json:
    install(input_value)
    logger = logger_for(input_value)
    request_messages: list[dict[str, Json]] = []
    messages = input_value.get("messages")
    if messages is not None:
        request_messages = messages
    response = await logger.try_short_circuit_search(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L291-L418
        model=input_value["model"],
        messages=request_messages,
        tools=input_value.get("tools"),
        custom_llm_provider=input_value.get("custom_llm_provider"),
        kwargs=kwargs_for(input_value),
    )
    return cast(Json, response)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L291-L418


async def execute_search(input_value: Input) -> Json:
    install(input_value)
    logger = logger_for(input_value)
    text, response = await logger._execute_search(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1523-L1624
        query=input_value["query"],
        kwargs=kwargs_for(input_value),
        rich=rich_value(input_value),
    )
    response_value: Json = None
    if response is not None:
        response_value = cast(Json, response.model_dump(mode="json"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/search/transformation.py#L64-L78
    return {"text": text, "response": response_value, "calls": RECORDED}


def pure_lookup(input_value: Input) -> Json:
    tool_inputs = input_value.get("tool_inputs")
    if tool_inputs is not None:
        return [
            cast(Json, WebSearchInterceptionLogger._rich_search_input(tool_input))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1481-L1507
            for tool_input in tool_inputs
        ]
    providers = input_value.get("search_providers")
    if providers is None:
        return []
    return [
        WebSearchInterceptionLogger._provider_supports_rich_search(provider)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L1509-L1521
        for provider in providers
    ]


def run(input_value: Input) -> Json:
    operation = input_value["operation"]
    if operation == "short_circuit":
        return asyncio.run(short_circuit(input_value))
    if operation == "execute_search":
        return asyncio.run(execute_search(input_value))
    return pure_lookup(input_value)
