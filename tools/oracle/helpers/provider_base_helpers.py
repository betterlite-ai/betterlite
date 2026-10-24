import functools
import json
import os
from collections.abc import Callable
from types import ModuleType, SimpleNamespace
from typing import TYPE_CHECKING, Protocol, TypeAlias, TypeVar, cast

import httpx
import litellm
from litellm.llms.base_llm import base_utils
from litellm.llms.base_llm.chat.transformation import BaseConfig, BaseLLMException
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.utils import (
    add_dummy_tool,
    any_assistant_message_has_thinking_blocks,
    has_tool_call_blocks,
    last_assistant_with_tool_calls_has_no_thinking_blocks,
)
from pydantic import BaseModel, ConfigDict

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Base = TypeVar("Base")


class ChatHelpers(Protocol):
    def is_thinking_enabled(self, non_default_params: Object) -> bool: ...
    def is_max_tokens_in_request(self, non_default_params: Object) -> bool: ...
    def update_optional_params_with_thinking_tokens(
        self, requested: Object, options: Object
    ) -> None: ...
    def _add_tools_to_optional_params(self, options: Object, tools: list[Json]) -> Object: ...


class FormatHelpers(Protocol):
    def type_to_response_format_param(
        self, response_format: Object | None, ref_template: str | None
    ) -> Json: ...


class ErrorView(Protocol):
    status_code: int
    status_code_is_synthesized: bool
    message: str
    headers: dict[str, str] | None
    request: httpx.Request
    response: httpx.Response
    body: dict[str, object] | None


class RequestLine(BaseModel):
    method: str
    url: str


class ResponseLine(BaseModel):
    status_code: int
    request: RequestLine


class ErrorParts(BaseModel):
    status_code: int
    message: str
    headers: list[tuple[str, str]] | None
    request: RequestLine | None
    response: ResponseLine | None
    body: dict[str, object] | None
    status_code_is_synthesized: bool


class HiddenModel(BaseModel):
    model_config = ConfigDict(extra="allow")


def to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def as_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def fixture_function(*_: object) -> None:
    return None


def concrete(base: type[Base]) -> Base:
    fixture = cast("type[Base]", type("Fixture", (base,), {}))
    setattr(fixture, "__abstractmethods__", frozenset())  # noqa: B010 - type has no typed attribute for it
    return fixture()


def check_environment(environment: Json) -> None:
    names = ("DEFAULT_MAX_TOKENS", "LITELLM_MODIFY_PARAMS")
    if {name: os.environ[name] for name in names if name in os.environ} != environment:
        raise ValueError("the call environment must match the case env")


def http_request(line: RequestLine) -> httpx.Request:
    return httpx.Request(line.method, line.url)


def base_error(argument: Json) -> Json:
    parts = ErrorParts.model_validate(argument)
    response = parts.response
    error = cast(
        "ErrorView",
        BaseLLMException(
            status_code=parts.status_code,
            message=parts.message,
            headers=None if parts.headers is None else dict(parts.headers),
            request=None if parts.request is None else http_request(parts.request),
            response=None
            if response is None
            else httpx.Response(response.status_code, request=http_request(response.request)),
            body=parts.body,
            status_code_is_synthesized=parts.status_code_is_synthesized,
        ),
    )
    inner = error.response.request
    return {
        "status_code": error.status_code,
        "status_code_is_synthesized": error.status_code_is_synthesized,
        "message": error.message,
        "headers": None if error.headers is None else to_json(list(error.headers.items())),
        "request": {"method": error.request.method, "url": str(error.request.url)},
        "response": {
            "status_code": error.response.status_code,
            "request": {"method": inner.method, "url": str(inner.url)},
        },
        "body": to_json(error.body),
    }


def class_config(argument: Json) -> Json:
    members: dict[str, object] = {
        "function": fixture_function,
        "builtin": len,
        "classmethod": classmethod(fixture_function),
        "staticmethod": staticmethod(fixture_function),
        "property": property(fixture_function),
        "callable": functools.partial(fixture_function),
    }
    fields = as_object(argument)
    attributes: dict[str, object] = dict(as_object(fields["values"]))
    methods = as_object(fields["methods"])
    attributes.update({name: members[str(kind)] for name, kind in methods.items()})
    parent = type("Parent", (BaseConfig,), {"inherited_default": 7})
    fixture = cast("type[BaseConfig]", type("Fixture", (parent,), attributes))
    return to_json(fixture.get_config())


def hidden_params(argument: Json) -> Json:
    fields = as_object(argument)
    hidden = fields["hidden"]
    values: dict[str, object] = {}
    if isinstance(hidden, dict):
        mapping = as_object(next(iter(hidden.values())))
        model = HiddenModel.model_validate(mapping)
        values["_hidden_params"] = model if "model" in hidden else mapping
    elif hidden == "opaque":
        values["_hidden_params"] = 5
    response = values if fields["kind"] == "dict" else SimpleNamespace(**values)
    projected = get_hidden_params_dict(response, create=fields["create"] is True)
    after: object = (values if isinstance(response, dict) else vars(response)).get("_hidden_params")
    if isinstance(after, BaseModel):
        after = cast("Json", json.loads(after.model_dump_json()))
    kept = to_json(after)
    return [to_json(projected), kept if isinstance(kept, dict) else None]


def thinking(argument: Json) -> Json:
    config = cast("ChatHelpers", concrete(BaseConfig))
    params = as_object(argument)
    enabled = config.is_thinking_enabled(params)
    return {"thinking": enabled, "max_tokens": config.is_max_tokens_in_request(params)}


def thinking_max_tokens(argument: Json) -> Json:
    fields = as_object(argument)
    check_environment(fields["environment"])
    options = as_object(fields["options"])
    config = cast("ChatHelpers", concrete(BaseConfig))
    config.update_optional_params_with_thinking_tokens(as_object(fields["requested"]), options)
    return options


def append_tools(argument: Json) -> Json:
    fields = as_object(argument)
    tools = fields["tools"]
    config = cast("ChatHelpers", concrete(BaseConfig))
    options = as_object(fields["options"])
    listed = tools if isinstance(tools, list) else []
    # LiteLLM keeps the helper private, and it is the function under test
    return config._add_tools_to_optional_params(options, listed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def response_format(argument: Json) -> Json:
    fields = as_object(argument)
    given = None if fields["format"] is None else as_object(fields["format"])
    template = fields["ref_template"]
    helpers = cast("FormatHelpers", cast("ModuleType", base_utils))
    named = template if isinstance(template, str) else None
    return helpers.type_to_response_format_param(given, named)


def history(argument: Json) -> Json:
    messages = cast("list[AllMessageValues]", argument)
    return [
        has_tool_call_blocks(messages),
        any_assistant_message_has_thinking_blocks(messages),
        last_assistant_with_tool_calls_has_no_thinking_blocks(messages),
    ]


def dummy_tool(_: Json) -> Json:
    tools = {json.dumps(add_dummy_tool(provider)) for provider in ("anthropic", "bedrock", "")}
    if len(tools) != 1:
        raise ValueError("add_dummy_tool depends on custom_llm_provider")
    return to_json(add_dummy_tool("anthropic"))


def modify_params(argument: Json) -> Json:
    fields = as_object(argument)
    check_environment(fields["environment"])
    configured = fields["configured"]
    if isinstance(configured, bool):
        litellm.modify_params = configured
    return litellm.modify_params


def responses_init(_: Json) -> Json:
    return to_json(sorted(vars(concrete(BaseResponsesAPIConfig))))


def image_edit_init(_: Json) -> Json:
    return to_json(sorted(vars(concrete(BaseImageEditConfig))))


HANDLERS: dict[str, Callable[[Json], Json]] = {
    "base_error": base_error,
    "class_config": class_config,
    "thinking": thinking,
    "thinking_max_tokens": thinking_max_tokens,
    "append_tools": append_tools,
    "response_format": response_format,
    "hidden_params": hidden_params,
    "history": history,
    "dummy_tool": dummy_tool,
    "modify_params": modify_params,
    "responses_init": responses_init,
    "image_edit_init": image_edit_init,
}


def run(input_value: dict[str, Json]) -> Json:
    name, argument = next(iter(input_value.items()))
    return HANDLERS[name](argument)
