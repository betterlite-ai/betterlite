import logging
from collections.abc import Iterator
from contextlib import contextmanager
from copy import deepcopy
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import litellm
from litellm.llms.bedrock.common_utils import bedrock_supports_tool_search
from litellm.llms.bedrock.messages.invoke_transformations.anthropic_claude3_transformation import (
    AmazonAnthropicClaudeMessagesConfig as MessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Object: TypeAlias = dict[str, JsonValue]
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)
MAX_TOKENS: Final = 2048
USER: Final[list[JsonValue]] = [{"role": "user", "content": "hi"}]
DROPPED: Final = "Bedrock Invoke: dropping unsupported anthropic-beta values from client headers"
STRIPPED: Final = "Bedrock Invoke: stripping unsupported top-level request fields: "
TEXTS = TypeAdapter(list[str])
FIXED_MODEL_COST: Final[Object] = {
    "bedrock/claude-opus-4-5": {"litellm_provider": "bedrock"},
    "bedrock/fixture-no-flag": {"litellm_provider": "bedrock", "mode": "chat"},
    "bedrock/fixture-tool-search": {"litellm_provider": "bedrock", "supports_tool_search": True},
}


class BetaCall(TypedDict):
    model: str
    messages: NotRequired[list[JsonValue]]
    tools: NotRequired[list[JsonValue]]
    mcp_servers: NotRequired[list[JsonValue]]
    safeguards: NotRequired[JsonValue]
    params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]
    outgoing: NotRequired[Object]
    injected: NotRequired[bool]
    subclass_provider: NotRequired[str | None]


class BetaHeaders(TypedDict):
    function: Literal["beta_headers"]
    model_cost: Object
    calls: list[BetaCall]


class ContextCase(TypedDict):
    request: Object
    betas: NotRequired[list[str]]


class ContextFilter(TypedDict):
    function: Literal["context_filter"]
    cases: list[ContextCase]


class WebSearch(TypedDict):
    function: Literal["web_search"]
    request: Object
    model: str


class FieldsCase(TypedDict):
    request: Object


class UnsupportedFields(TypedDict):
    function: Literal["unsupported_fields"]
    cases: list[FieldsCase]


class IdentityCase(TypedDict):
    model: str
    subclass_provider: NotRequired[str | None]


class ProviderIdentity(TypedDict):
    function: Literal["provider_identity"]
    model_cost: Object
    cases: list[IdentityCase]


Input: TypeAlias = BetaHeaders | ContextFilter | WebSearch | UnsupportedFields | ProviderIdentity


class Watch(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.warnings: list[str] = []
        self.debugs: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if record.levelno == logging.WARNING:
            self.warnings.append(message)
        if record.levelno == logging.DEBUG:
            self.debugs.append(message)


@contextmanager
def watching() -> Iterator[Watch]:
    watch = Watch()
    logger = logging.getLogger("LiteLLM")
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [watch]
    logger.setLevel(logging.DEBUG)
    try:
        yield watch
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)


@runtime_checkable
class MessagesView(Protocol):
    def transform_anthropic_messages_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[JsonValue],
        params: Object,
        extra: GenericLiteLLMParams,
        headers: dict[str, str],
    ) -> Object: ...


VIEW: Final[TypeAdapter[MessagesView]] = TypeAdapter(MessagesView, config=ARBITRARY)


class Probe(MessagesConfig):
    requested_provider: str | None = None

    @property
    @override
    def custom_llm_provider(self) -> str | None:
        return self.requested_provider

    def invoke(
        self, model: str, messages: list[JsonValue], options: Object, headers: dict[str, str]
    ) -> Object:
        view: MessagesView = VIEW.validate_python(self)
        return view.transform_anthropic_messages_request(
            model, messages, options, GenericLiteLLMParams(), headers
        )


def probe(provider: str | None) -> Probe:
    created = Probe()
    created.requested_provider = provider
    return created


def effective_provider(given: str | None) -> str:
    if given is None or given == "":
        return "bedrock"
    return given


def beta_options(call: BetaCall) -> Object:
    options: Object = {}
    if "params" in call:
        options.update(call["params"])
    if "tools" in call:
        options["tools"] = call["tools"]
    if "mcp_servers" in call:
        options["mcp_servers"] = call["mcp_servers"]
    if "safeguards" in call:
        options["safeguards"] = call["safeguards"]
    if "outgoing" in call:
        outgoing = call["outgoing"]
        if "context_management" in outgoing:
            options["context_management"] = outgoing["context_management"]
        if "thinking" in outgoing:
            options["thinking"] = outgoing["thinking"]
    if "max_tokens" not in options:
        options["max_tokens"] = MAX_TOKENS
    return options


def beta_call(call: BetaCall) -> JsonValue:
    given = call["subclass_provider"] if "subclass_provider" in call else None
    messages: list[JsonValue] = call["messages"] if "messages" in call else []
    headers: dict[str, str] = call["headers"] if "headers" in call else {}
    with watching() as watch:
        body = probe(effective_provider(given)).invoke(
            call["model"], deepcopy(messages), beta_options(call), headers
        )
    raw = body.get("anthropic_beta")
    return {
        "betas": [] if raw is None else raw,
        "warnings": [message for message in watch.warnings if message.startswith(DROPPED)],
    }


def beta_headers(call: BetaHeaders) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    return {"calls": [beta_call(entry) for entry in call["calls"]]}


def betas_of(body: Object) -> set[str]:
    raw = body.get("anthropic_beta")
    if raw is None:
        return set()
    return set(TEXTS.validate_python(raw))


def context_case(case: ContextCase) -> JsonValue:
    options = dict(case["request"])
    if "max_tokens" not in options:
        options["max_tokens"] = MAX_TOKENS
    without = dict(options)
    if "context_management" in without:
        del without["context_management"]
    model = "fixture-tool-search"
    with_body = probe("bedrock").invoke(model, deepcopy(USER), options, {})
    without_body = probe("bedrock").invoke(model, deepcopy(USER), without, {})
    kept = dict(case["request"])
    if "context_management" in with_body:
        kept["context_management"] = with_body["context_management"]
    elif "context_management" in kept:
        del kept["context_management"]
    given = case["betas"] if "betas" in case else []
    added = betas_of(with_body) - betas_of(without_body)
    return {"request": kept, "betas": sorted(set(given) | added)}


def context_filter(call: ContextFilter) -> JsonValue:
    litellm.model_cost = FIXED_MODEL_COST
    return {"cases": [context_case(entry) for entry in call["cases"]]}


def web_search(call: WebSearch) -> JsonValue:
    litellm.model_cost = FIXED_MODEL_COST
    request = call["request"]
    options: Object = {"max_tokens": MAX_TOKENS}
    if "tools" in request:
        options["tools"] = request["tools"]
    probe("bedrock").invoke(call["model"], deepcopy(USER), options, {})
    return {"accepted": True}


def fields_case(case: FieldsCase) -> JsonValue:
    request = case["request"]
    options = dict(request)
    if "max_tokens" not in options:
        options["max_tokens"] = MAX_TOKENS
    with watching() as watch:
        body = probe("bedrock").invoke("fixture-no-flag", deepcopy(USER), options, {})
    return {
        "request": {name: body[name] for name in request if name in body},
        "logs": [message for message in watch.debugs if message.startswith(STRIPPED)],
    }


def unsupported_fields(call: UnsupportedFields) -> JsonValue:
    litellm.model_cost = FIXED_MODEL_COST
    return {"cases": [fields_case(entry) for entry in call["cases"]]}


def provider_identity(call: ProviderIdentity) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    answers: list[JsonValue] = []
    for entry in call["cases"]:
        given = entry["subclass_provider"] if "subclass_provider" in entry else None
        answers.append(
            {
                "custom_llm_provider": MessagesConfig().custom_llm_provider,
                "beta_headers_provider": probe(given).beta_headers_provider,
                "supports_tool_search": bedrock_supports_tool_search(entry["model"]),
            }
        )
    return {"cases": answers}


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "beta_headers":
            return beta_headers(input_value)
        case "context_filter":
            return context_filter(input_value)
        case "web_search":
            return web_search(input_value)
        case "unsupported_fields":
            return unsupported_fields(input_value)
        case "provider_identity":
            return provider_identity(input_value)
