from typing import Final, Literal, Protocol, TypedDict

import httpx
import litellm
from litellm.llms.maritalk import MaritalkConfig
from litellm.llms.meta_llama.chat.transformation import LlamaAPIConfig
from litellm.llms.moonshot.chat.transformation import MoonshotChatConfig
from litellm.llms.morph.chat.transformation import MorphChatConfig
from litellm.llms.nadir.chat.transformation import (  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nadir/chat/transformation.py#L22-L30
    NadirConfig,
    _reported_cost_usd,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nadir/chat/transformation.py#L22-L33
)
from litellm.llms.nebius.chat.transformation import NebiusConfig
from litellm.llms.novita.chat.transformation import NovitaConfig
from litellm.llms.nscale.chat.transformation import NscaleConfig
from litellm.llms.v0.chat.transformation import V0ChatConfig
from litellm.llms.wandb.chat.transformation import WandbConfig
from litellm.llms.zai.chat.transformation import ZAIChatConfig
from litellm.types.llms.openai import AllMessageValues, ChatCompletionToolParam
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
litellm.suppress_debug_info = True

Provider = Literal[
    "maritalk",
    "llama",
    "moonshot",
    "morph",
    "nadir",
    "nebius",
    "novita",
    "nscale",
    "v0",
    "wandb",
    "zai",
]


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[JsonValue]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        /,
        *,
        stream: bool | None = None,
    ) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
        /,
    ) -> dict[str, str]: ...

    # ast-grep-ignore: no-long-names
    def remove_cache_control_flag_from_messages_and_tools(
        self,
        model: str,
        messages: list[AllMessageValues],
        tools: list[ChatCompletionToolParam] | None = None,
        /,
    ) -> tuple[list[AllMessageValues], list[ChatCompletionToolParam] | None]: ...


class SupportedCall(TypedDict):
    provider: Provider
    model: str
    reasoning: bool
    chat_models: list[str]
    text_models: list[str]


class ParamsCall(TypedDict):
    provider: Provider
    model: str
    reasoning: bool
    requested: dict[str, JsonValue]
    existing: dict[str, JsonValue]
    chat_models: list[str]
    text_models: list[str]


class UrlCall(TypedDict):
    api_base: str | None


class RequestCall(TypedDict):
    model: str
    reasoning: bool
    messages: list[AllMessageValues]
    optional_params: dict[str, JsonValue]


class CostCall(TypedDict):
    body: str


class NovitaCall(TypedDict):
    api_key: str | None
    headers: dict[str, str]


class CacheCall(TypedDict):
    model: str
    messages: list[AllMessageValues]
    tools: list[ChatCompletionToolParam] | None


class ErrorCall(TypedDict):
    message: str
    status: int
    headers: dict[str, str]


class Input(TypedDict):
    supported: list[SupportedCall]
    params: list[ParamsCall]
    moonshot_urls: list[UrlCall]
    moonshot_requests: list[RequestCall]
    nadir_cost: list[CostCall]
    novita_headers: list[NovitaCall]
    zai_cache: list[CacheCall]
    maritalk_errors: list[ErrorCall]


CONFIGS: Final[dict[Provider, type[ChatConfig]]] = {
    "maritalk": MaritalkConfig,
    "llama": LlamaAPIConfig,
    "moonshot": MoonshotChatConfig,
    "morph": MorphChatConfig,
    "nadir": NadirConfig,
    "nebius": NebiusConfig,
    "novita": NovitaConfig,
    "nscale": NscaleConfig,
    "v0": V0ChatConfig,
    "wandb": WandbConfig,
    "zai": ZAIChatConfig,
}


def failure() -> JsonValue:
    answer: dict[str, JsonValue] = {"ok": False, "value": None, "status": None}
    return answer


def supported(call: SupportedCall) -> JsonValue:
    config = CONFIGS[call["provider"]]()
    params = config.get_supported_openai_params(call["model"])
    return {"ok": True, "value": params, "status": None}


def params(call: ParamsCall) -> JsonValue:
    config = CONFIGS[call["provider"]]()
    mapped = config.map_openai_params(
        dict(call["requested"]), dict(call["existing"]), call["model"], drop_params=False
    )
    return {"ok": True, "value": JSON.validate_python(mapped), "status": None}


def url(call: UrlCall) -> JsonValue:
    config = CONFIGS["moonshot"]()
    resolved = config.get_complete_url(call["api_base"], None, "moonshot/m", {}, {})
    return {"ok": True, "value": resolved, "status": None}


def request(call: RequestCall) -> JsonValue:
    config = CONFIGS["moonshot"]()
    body = config.transform_request(
        call["model"],
        list(call["messages"]),
        dict(call["optional_params"]),
        {"custom_llm_provider": "moonshot"},
        {},
    )
    return {"ok": True, "value": JSON.validate_python(body), "status": None}


def cost(call: CostCall) -> JsonValue:
    response = httpx.Response(200, content=call["body"].encode())
    reported = _reported_cost_usd(response)
    return {"ok": True, "value": JSON.validate_python(reported), "status": None}


def novita(call: NovitaCall) -> JsonValue:
    config = CONFIGS["novita"]()
    try:
        headers = config.validate_environment(
            dict(call["headers"]), "novita/m", [], {}, {}, call["api_key"], None
        )
    except ValueError:
        return failure()
    return {"ok": True, "value": JSON.validate_python(headers), "status": None}


def cache(call: CacheCall) -> JsonValue:
    config = CONFIGS["zai"]()
    messages, tools = config.remove_cache_control_flag_from_messages_and_tools(
        call["model"], list(call["messages"]), call["tools"]
    )
    value: dict[str, JsonValue] = {
        "messages": JSON.validate_python(messages),
        "tools": JSON.validate_python(tools),
    }
    return {"ok": True, "value": value, "status": None}


def maritalk(call: ErrorCall) -> JsonValue:
    error = MaritalkConfig().get_error_class(call["message"], call["status"], call["headers"])
    value: dict[str, JsonValue] = {
        "status_code": error.status_code,
        "message": error.message,
        "headers": JSON.validate_python(call["headers"]),
    }
    return {"ok": True, "value": value, "status": None}


def run(value: Input) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "supported": [supported(call) for call in value["supported"]],
        "params": [params(call) for call in value["params"]],
        "moonshot_urls": [url(call) for call in value["moonshot_urls"]],
        "moonshot_requests": [request(call) for call in value["moonshot_requests"]],
        "nadir_cost": [cost(call) for call in value["nadir_cost"]],
        "novita_headers": [novita(call) for call in value["novita_headers"]],
        "zai_cache": [cache(call) for call in value["zai_cache"]],
        "maritalk_errors": [maritalk(call) for call in value["maritalk_errors"]],
    }
    return JSON.validate_python(answer)
