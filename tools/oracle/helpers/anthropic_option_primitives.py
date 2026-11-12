import os
from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.anthropic.chat import transformation
from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from litellm.types.llms.openai import OpenAIWebSearchOptions

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class OptionConfig(Protocol):
    custom_llm_provider: str | None
    _resolved_provider: str

    def __init__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L307-L320
        self,
        max_tokens: int | None = ...,
        stop_sequences: list[str] | None = ...,
        temperature: int | None = ...,
        top_p: int | None = ...,
        top_k: int | None = ...,
        metadata: dict[str, Json] | None = ...,
        system: str | None = ...,
    ) -> None: ...

    @classmethod
    def get_config(cls, *, model: str | None = ...) -> dict[str, Json]: ...

    @staticmethod
    def get_max_tokens_for_model(model: str | None = ...) -> int: ...

    def map_web_search_tool(self, value: OpenAIWebSearchOptions) -> dict[str, Json]: ...

    def _map_stop_sequences(self, stop: str | list[str] | None) -> list[str] | None:
        ...


# LiteLLM's constructor leaves list and dict bare, so one cast bridges to the typed view
CONFIG: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L307-L320
    "type[OptionConfig]",
    AnthropicConfig,
)
CONTEXT_MAPPER: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1416-L1471
    "Callable[[list[dict[str, Json]] | dict[str, Json]], dict[str, Json] | None]",
    AnthropicConfig.map_openai_context_management_to_anthropic,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1416-L1471
)


class Defaults(TypedDict, total=False):
    max_tokens: int
    stop_sequences: list[str]
    temperature: int
    top_p: int
    top_k: int
    metadata: dict[str, Json]
    system: str


class DefaultsCall(TypedDict):
    function: Literal["defaults"]
    defaults: Defaults
    model: str | None
    default_chat_max_tokens: int
    configured_provider: str | None


class CapacityCall(TypedDict):
    function: Literal["model_output_capacity"]
    models: list[str | None]
    default_chat_max_tokens: int


class StopCall(TypedDict):
    function: Literal["stop_sequence_filter"]
    drop_params: bool
    stops: list[Json]


class WebSearchCall(TypedDict):
    function: Literal["web_search_context_size"]
    values: list[OpenAIWebSearchOptions]


class ContextCall(TypedDict):
    function: Literal["context_compaction_edit"]
    values: list[Json]


class UserIdCall(TypedDict):
    function: Literal["user_identifier_filter"]
    ids: list[str]


Call: TypeAlias = (
    DefaultsCall | CapacityCall | StopCall | WebSearchCall | ContextCall | UserIdCall
)


class Input(TypedDict):
    calls: list[Call]


def check_environment(default_chat_max_tokens: int) -> None:
    configured = int(os.environ["DEFAULT_ANTHROPIC_CHAT_MAX_TOKENS"])
    if configured != default_chat_max_tokens:
        raise ValueError("the case env must match default_chat_max_tokens")


def provider_instance(configured: str | None) -> OptionConfig:
    if configured is None:
        return CONFIG()
    attributes: dict[str, str | None] = {"custom_llm_provider": configured}
    fixture = cast("type[OptionConfig]", type("ProviderFixture", (CONFIG,), attributes))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L307-L320
    return fixture()


def defaults_answer(call: DefaultsCall) -> Json:
    check_environment(call["default_chat_max_tokens"])
    attributes: dict[str, str | None] = {}
    config_class = cast("type[OptionConfig]", type("ConfigFixture", (CONFIG,), attributes))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L307-L320
    config_class(**call["defaults"])
    config = config_class.get_config(model=call["model"])
    instance = provider_instance(call["configured_provider"])
    return {
        "config": config,
        "custom_llm_provider": instance.custom_llm_provider,
        "resolved_provider": instance._resolved_provider,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L326-L328
    }


def capacity_answer(call: CapacityCall) -> Json:
    check_environment(call["default_chat_max_tokens"])
    values: list[Json] = [
        CONFIG.get_max_tokens_for_model(model) for model in call["models"]
    ]
    return {"values": values}


def stop_answers(call: StopCall) -> Json:
    litellm.drop_params = call["drop_params"]
    answers: list[Json] = []
    for stop in call["stops"]:
        answer: Json = {"raised": True}
        try:
            mapped = CONFIG()._map_stop_sequences(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1230-L1248
                cast("str | list[str] | None", stop)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1230-L1248
            )
        except AttributeError:
            pass
        else:
            mapped_value: Json = None
            if mapped is not None:
                kept: list[Json] = []
                kept.extend(mapped)
                mapped_value = kept
            answer = {"value": mapped_value}
        answers.append(answer)
    return {"values": answers}


def web_search_answers(call: WebSearchCall) -> Json:
    answers: list[Json] = []
    for options in call["values"]:
        answer: Json = {"raised": True}
        try:
            tool = CONFIG().map_web_search_tool(options)
        except KeyError:
            pass
        else:
            answer = {"value": tool}
        answers.append(answer)
    return {"values": answers}


def context_answers(call: ContextCall) -> Json:
    answers: list[Json] = []
    for value in call["values"]:
        parameter = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L1416-L1471
            "list[dict[str, Json]] | dict[str, Json]", value
        )
        mapped = CONTEXT_MAPPER(parameter)
        answers.append({"value": mapped})
    return {"values": answers}


def user_id_answer(call: UserIdCall) -> Json:
    values: list[Json] = [
        transformation._valid_user_id(user_id)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L2795-L2808
        for user_id in call["ids"]
    ]
    return {"values": values}


def answer(call: Call) -> Json:
    match call["function"]:
        case "defaults":
            return defaults_answer(call)
        case "model_output_capacity":
            return capacity_answer(call)
        case "stop_sequence_filter":
            return stop_answers(call)
        case "web_search_context_size":
            return web_search_answers(call)
        case "context_compaction_edit":
            return context_answers(call)
        case "user_identifier_filter":
            return user_id_answer(call)


def run(input_value: Input) -> Json:
    answers: list[Json] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
