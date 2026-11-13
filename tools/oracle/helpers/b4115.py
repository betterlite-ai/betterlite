import asyncio
from typing import Literal, Required, TypedDict

import litellm
from litellm.integrations.websearch_interception.handler import WebSearchInterceptionLogger
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.proxy import proxy_server
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.router import Router
from litellm.types.integrations.websearch_interception import RichWebSearchInput
from litellm.types.router import SearchToolTypedDict
from litellm.types.utils import LlmProviders
from openai import APIError
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MOCK_MODEL = "gpt-4o"
MOCK_MESSAGES: list[dict[str, JsonValue]] = [{"role": "user", "content": "mock"}]


class SearchError(TypedDict):
    kind: Literal["rate_limit", "context_window", "unavailable"]
    message: str


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
    messages: list[dict[str, JsonValue]]
    tools: list[dict[str, JsonValue]]
    custom_llm_provider: str | None
    enabled_providers: list[str]
    search_tools: list[SearchToolTypedDict]
    search_tool_name: str
    search_response: dict[str, JsonValue]
    search_error: SearchError
    query: Required[str]
    tool_inputs: list[JsonValue]
    search_providers: list[str | None]
    caller: bool


RECORDED: list[JsonValue] = []
SCRIPTED_RESPONSE: list[SearchResponse] = []
SCRIPTED_ERROR: list[BaseException] = []


def response_value(response: SearchResponse | None) -> JsonValue:
    if response is None:
        return None
    results: list[JsonValue] = [
        {
            "title": result.title,
            "url": result.url,
            "snippet": result.snippet,
            "date": result.date,
            "last_updated": result.last_updated,
        }
        for result in response.results
    ]
    return {"results": results, "object": response.object}


class ScriptedLogger(WebSearchInterceptionLogger):
    async def short_circuit(
        self,
        model: str,
        messages: list[dict[str, JsonValue]],
        tools: list[dict[str, JsonValue]] | None,
        provider: str | None,
        kwargs: dict[str, JsonValue] | None,
    ) -> JsonValue:
        response = await self.try_short_circuit_search(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/websearch_interception/handler.py#L291-L418
            model=model,
            messages=messages,
            tools=tools,
            custom_llm_provider=provider,
            kwargs=kwargs,
        )
        return JSON.validate_python(response)

    async def search(
        self,
        query: str,
        rich: RichWebSearchInput | None,
        kwargs: dict[str, JsonValue] | None,
    ) -> JsonValue:
        text, response = await self._execute_search(
            query=query,
            kwargs=kwargs,
            rich=rich,
        )
        return {"text": text, "response": response_value(response), "calls": RECORDED}

    def first_rich_input(self, tool_inputs: list[JsonValue]) -> RichWebSearchInput | None:
        if not tool_inputs:
            return None
        return self._rich_search_input(tool_inputs[0])

    def rich_jsons(self, tool_inputs: list[JsonValue]) -> list[JsonValue]:
        return [
            JSON.validate_python(self._rich_search_input(tool_input))
            for tool_input in tool_inputs
        ]

    def supports_rich(self, providers: list[str | None]) -> list[JsonValue]:
        return [
            self._provider_supports_rich_search(provider)
            for provider in providers
        ]


async def fake_asearch(
    query: str | list[JsonValue], search_provider: str, **kwargs: JsonValue
) -> SearchResponse:
    params = dict(kwargs)
    metadata = params.pop("litellm_metadata", None)
    call: dict[str, JsonValue] = {
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


async def deny_key(search_tool_name: str, valid_token: UserAPIKeyAuth) -> Literal[True]:
    raise ProxyException(
        message=f"the key for user {valid_token.user_id} may not call {search_tool_name}",
        type="auth_error",
        param=None,
        code=401,
    )


def scripted_error(spec: SearchError) -> BaseException:
    if spec["kind"] == "unavailable":
        return ValueError(spec["message"])
    try:
        if spec["kind"] == "rate_limit":
            litellm.completion(
                model=MOCK_MODEL,
                messages=MOCK_MESSAGES,
                mock_response="litellm.RateLimitError",
            )
        else:
            litellm.completion(
                model=MOCK_MODEL,
                messages=MOCK_MESSAGES,
                mock_response="litellm.ContextWindowExceededError",
            )
    except APIError as error:
        return error
    raise ValueError("the mock completion did not raise")


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
    litellm.asearch = fake_asearch
    tools = input_value.get("search_tools")
    if tools is not None:
        proxy_server.llm_router = Router(model_list=[], search_tools=tools)
    if input_value.get("caller"):
        auth_checks.can_key_call_search_tool = deny_key


def logger_for(input_value: Input) -> ScriptedLogger:
    enabled: list[LlmProviders | str] = []
    providers = input_value.get("enabled_providers")
    if providers is not None:
        enabled = list(providers)
    return ScriptedLogger(
        enabled_providers=enabled,
        search_tool_name=input_value.get("search_tool_name"),
    )


def kwargs_for(input_value: Input) -> dict[str, JsonValue] | None:
    if not input_value.get("caller"):
        return None
    caller: dict[str, JsonValue] = {"team_id": None}
    metadata: dict[str, JsonValue] = {"user_api_key_auth": caller}
    kwargs: dict[str, JsonValue] = {"metadata": metadata}
    return kwargs


def rich_value(logger: ScriptedLogger, input_value: Input) -> RichWebSearchInput | None:
    tool_inputs = input_value.get("tool_inputs")
    if tool_inputs is None:
        return None
    return logger.first_rich_input(tool_inputs)


async def short_circuit(input_value: Input) -> JsonValue:
    install(input_value)
    logger = logger_for(input_value)
    request_messages: list[dict[str, JsonValue]] = []
    messages = input_value.get("messages")
    if messages is not None:
        request_messages = messages
    return await logger.short_circuit(
        model=input_value["model"],
        messages=request_messages,
        tools=input_value.get("tools"),
        provider=input_value.get("custom_llm_provider"),
        kwargs=kwargs_for(input_value),
    )


async def execute_search(input_value: Input) -> JsonValue:
    install(input_value)
    logger = logger_for(input_value)
    rich = rich_value(logger, input_value)
    return await logger.search(input_value["query"], rich, kwargs_for(input_value))


def pure_lookup(input_value: Input) -> JsonValue:
    logger = logger_for(input_value)
    tool_inputs = input_value.get("tool_inputs")
    if tool_inputs is not None:
        return logger.rich_jsons(tool_inputs)
    providers = input_value.get("search_providers")
    if providers is None:
        return []
    return logger.supports_rich(providers)


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    if operation == "short_circuit":
        return asyncio.run(short_circuit(input_value))
    if operation == "execute_search":
        return asyncio.run(execute_search(input_value))
    return pure_lookup(input_value)
