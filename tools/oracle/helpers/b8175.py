from collections.abc import Callable
from typing import Protocol, TypeAlias, cast

import httpx
from litellm.llms.volcengine.chat.transformation import VolcEngineChatConfig
from litellm.llms.volcengine.common_utils import (
    get_volcengine_base_url,
    get_volcengine_headers,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.llms.volcengine.embedding.transformation import VolcEngineEmbeddingConfig
from litellm.types.llms.openai import AllEmbeddingInputValues
from litellm.types.utils import EmbeddingResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

# the eleven class values VolcEngineChatConfig declares and its constructor can set
CHAT_CONFIG_KEYS: tuple[str, ...] = (
    "frequency_penalty",
    "function_call",
    "functions",
    "logit_bias",
    "max_tokens",
    "n",
    "presence_penalty",
    "stop",
    "temperature",
    "top_p",
    "response_format",
)


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def get_config(self) -> dict[str, Json]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, Json]: ...


class Reply(Protocol):
    def model_dump(self) -> dict[str, Json]: ...


class ApiError(Protocol):
    status_code: int
    message: str
    headers: dict[str, str] | None


class EmbeddingConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, Json]: ...

    def get_complete_url(  # noqa: PLR0913 - mirrors LiteLLM's parameter list
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        *,
        stream: bool | None = None,
    ) -> str: ...

    def transform_embedding_request(
        self,
        model: str,
        input_values: AllEmbeddingInputValues,
        optional_params: dict[str, Json],
        headers: dict[str, str],
    ) -> dict[str, Json]: ...

    def validate_environment(  # noqa: PLR0913 - mirrors LiteLLM's parameter list
        self,
        headers: dict[str, str],
        model: str,
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        *,
        api_key: str | None = None,
    ) -> dict[str, str]: ...

    def transform_embedding_response(  # noqa: PLR0913 - mirrors LiteLLM's parameter list
        self,
        *,
        model: str,
        raw_response: httpx.Response,
        model_response: EmbeddingResponse,
        logging_obj: object,
        api_key: str | None,
        request_data: dict[str, Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
    ) -> Reply: ...

    def get_error_class(
        self,
        error_message: str,
        status_code: int,
        headers: dict[str, str],
    ) -> ApiError: ...


class Logging:
    def pre_call(self, **params: Json) -> None:
        del params

    def post_call(self, **params: Json) -> None:
        del params


def chat_config() -> ChatConfig:
    return cast("ChatConfig", VolcEngineChatConfig())


def embedding_config() -> EmbeddingConfig:
    return cast("EmbeddingConfig", VolcEngineEmbeddingConfig())


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def obj_or_empty(value: Json | None) -> Object:
    return {} if value is None else obj(value)


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def expect_int(fields: Object, name: str) -> int:
    value = fields.get(name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{name} must be an integer")
    return value


def optional_headers(fields: Object, name: str) -> dict[str, str] | None:
    value = fields.get(name)
    if value is None:
        return None
    headers: dict[str, str] = {}
    for key, item in obj(value).items():
        if not isinstance(item, str):
            raise TypeError(f"{name} values must be strings")
        headers[key] = item
    return headers


def chat_supported_params(_step: Object) -> Json:
    return cast("Json", chat_config().get_supported_openai_params(""))


def chat_defaults(step: Object) -> Json:
    defaults = obj(step.get("defaults"))
    if any(name not in CHAT_CONFIG_KEYS for name in defaults):
        return {"ok": False}
    for name in CHAT_CONFIG_KEYS:
        setattr(VolcEngineChatConfig, name, defaults.get(name))
    return cast("Json", chat_config().get_config())


def chat_map(step: Object) -> Json:
    params = obj(step.get("params"))
    options = obj_or_empty(step.get("optional_params"))
    try:
        return cast(
            "Json",
            chat_config().map_openai_params(params, options, "", drop_params=False),
        )
    except TypeError:
        return {"ok": False}


def embedding_supported_params(_step: Object) -> Json:
    return cast("Json", embedding_config().get_supported_openai_params(""))


def embedding_map(step: Object) -> Json:
    params = obj(step.get("params"))
    options = obj_or_empty(step.get("optional_params"))
    drop_params = step.get("drop_params")
    if drop_params is None:
        drop_params = False
    if not isinstance(drop_params, bool):
        raise TypeError("drop_params must be a boolean")
    try:
        return cast(
            "Json",
            embedding_config().map_openai_params(
                params, options, "", drop_params=drop_params
            ),
        )
    except ValueError:
        return {"ok": False}


def embedding_url(step: Object) -> Json:
    return embedding_config().get_complete_url(
        optional_text(step, "api_base"), None, "", {}, {}, stream=None
    )


def embedding_body(step: Object) -> Json:
    return cast(
        "Json",
        embedding_config().transform_embedding_request(
            text(step, "model"),
            cast("AllEmbeddingInputValues", step.get("input")),
            obj_or_empty(step.get("optional_params")),
            {},
        ),
    )


def embedding_headers(step: Object) -> Json:
    try:
        return cast(
            "Json",
            embedding_config().validate_environment(
                {}, "", [], {}, {}, api_key=optional_text(step, "api_key")
            ),
        )
    except ValueError:
        return {"ok": False}


def embedding_reply(step: Object) -> Json:
    response = httpx.Response(
        200,
        content=text(step, "body").encode(),
        request=httpx.Request("POST", "https://host"),
    )
    try:
        result = embedding_config().transform_embedding_response(
            model=text(step, "model"),
            raw_response=response,
            model_response=EmbeddingResponse(),
            logging_obj=Logging(),
            api_key=None,
            request_data={},
            optional_params={},
            litellm_params={},
        )
    except Exception:  # noqa: BLE001 - the wire contract carries every helper exception
        return {"ok": False}
    return {"ok": True, "reply": result.model_dump()}


def base_url(step: Object) -> Json:
    return get_volcengine_base_url(optional_text(step, "api_base"))


def headers(step: Object) -> Json:
    return cast(
        "Json",
        get_volcengine_headers(
            text(step, "api_key"), optional_headers(step, "extra_headers")
        ),
    )


def error_class(step: Object) -> Json:
    error = embedding_config().get_error_class(
        text(step, "message"),
        expect_int(step, "status"),
        optional_headers(step, "headers") or {},
    )
    value = error.headers
    rendered: Json = None
    if isinstance(value, httpx.Headers):
        rendered = cast("Json", dict(value))
    elif value is not None:
        rendered = cast("Json", value)
    return {
        "status_code": error.status_code,
        "message": error.message,
        "headers": rendered,
    }


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "chat_supported_params": chat_supported_params,
    "chat_defaults": chat_defaults,
    "chat_map": chat_map,
    "embedding_supported_params": embedding_supported_params,
    "embedding_map": embedding_map,
    "embedding_url": embedding_url,
    "embedding_body": embedding_body,
    "embedding_headers": embedding_headers,
    "embedding_reply": embedding_reply,
    "base_url": base_url,
    "headers": headers,
    "error_class": error_class,
}


def execute(step: Object) -> Json:
    kind = text(step, "kind")
    found = HANDLERS.get(kind)
    if found is None:
        raise ValueError(f"unknown step kind {kind}")
    return found(step)


def run(value: dict[str, Json]) -> Json:
    steps = value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    return [execute(obj(step)) for step in steps]
