import datetime as dt
from collections.abc import Callable
from typing import Literal, ParamSpec, Protocol, TypedDict, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.databricks.chat.transformation import DatabricksConfig
from litellm.types.utils import CallTypes, ModelResponse, ModelResponseStream
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

JSON_ADAPTER = TypeAdapter(JsonValue)
FAILURES = ("DatabricksException", "BadRequestError", "ImageFetchError")
MODEL = "databricks-meta-llama-3-1-70b"
Object = dict[str, JsonValue]
Params = ParamSpec("Params")
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class UrlCase(TypedDict):
    model: str
    api_base: str | None


class HeadersCase(TypedDict):
    model: str
    api_key: str | None
    api_base: str | None
    headers: dict[str, str] | None
    optional_params: Object
    user_agent: str | None


class ParamsCase(TypedDict):
    model: str
    non_default_params: Object
    optional_params: Object


class MessagesCase(TypedDict):
    model: str
    messages: list[Object]


class ResponseCase(TypedDict):
    body: Object
    json_mode: bool


class StreamCase(TypedDict):
    chunks: list[Object]
    json_mode: bool


class Input(TypedDict, total=False):
    function: Literal["url", "headers", "params", "messages", "response", "stream"]
    cases: list[JsonValue]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int


@runtime_checkable
class StreamIterator(Protocol):
    def chunk_parser(self, chunk: Object) -> ModelResponseStream: ...


# LiteLLM's Databricks members take bare dicts, so this protocol restates the types the cases use
@runtime_checkable
class ChatApi(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
        stream: bool | None = None,
    ) -> str: ...

    def validate_environment(
        self,
        headers: Object,
        model: str,
        messages: list[Object],
        optional_params: Object,
        litellm_params: Object,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> Object: ...

    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...

    def transform_request(
        self,
        model: str,
        messages: list[Object],
        optional_params: Object,
        litellm_params: Object,
        headers: Object,
    ) -> Object: ...

    def transform_response(
        self,
        _model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: Logging,
        _request_data: Object,
        _messages: list[Object],
        _optional_params: Object,
        _litellm_params: Object,
        _encoding: None,
        _api_key: str | None,
        /,
        *,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...

    def get_model_response_iterator(
        self, streaming_response: None, sync_stream: bool, json_mode: bool
    ) -> StreamIterator: ...


CHAT: ChatApi = TypeAdapter(ChatApi, config=ARBITRARY).validate_python(DatabricksConfig())


def logging_object() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type=CallTypes.completion,
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="b5841",
        function_id="b5841",
    )


def cases_of(input_value: Input) -> list[JsonValue]:
    cases = input_value.get("cases")
    if cases is None:
        raise ValueError("cases is required")
    return cases


def guarded(call: Callable[Params, Object], *args: Params.args, **kwargs: Params.kwargs) -> Object:
    try:
        return call(*args, **kwargs)
    except Exception as error:
        if type(error).__name__ not in FAILURES:
            raise
        failure = Failure.model_validate(error)
        return {"status": failure.status_code}
