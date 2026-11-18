import os
from datetime import UTC, datetime
from typing import Final, NotRequired, Protocol, TypeAlias, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.search.transformation import (
    CONNECTION_ID_ENV,
    MODEL_ENV,
    PROJECT_ENDPOINT_ENV,
    TOKEN_ENV,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.types.utils import CallTypes
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import TypedDict

Params: TypeAlias = dict[str, JsonValue]


class MinterSpec(TypedDict):
    token: str
    error: NotRequired[str | None]


class RequestScenario(TypedDict):
    environment: dict[str, str]
    query: str | list[str]
    optional_params: NotRequired[Params]


class RequestInput(TypedDict):
    scenarios: list[RequestScenario]


class HeaderScenario(TypedDict):
    environment: dict[str, str]
    headers: dict[str, str]
    minter: MinterSpec
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class HeaderInput(TypedDict):
    scenarios: list[HeaderScenario]


class ReplyScenario(TypedDict):
    environment: dict[str, str]
    body: str
    status: NotRequired[int | None]
    headers: NotRequired[dict[str, str]]
    optional_params: NotRequired[Params | None]


class ReplyInput(TypedDict):
    scenarios: list[ReplyScenario]


class CapScenario(TypedDict):
    environment: dict[str, str]
    query: str | list[str]
    body: str
    optional_params: NotRequired[Params]
    status: NotRequired[int | None]


class MaxResultsInput(TypedDict):
    scenarios: list[CapScenario]


class UrlScenario(TypedDict):
    environment: dict[str, str]
    api_base: NotRequired[str | None]


class SettingsInput(TypedDict):
    urls: list[UrlScenario]
    requests: list[RequestScenario]


class Input(TypedDict, total=False):
    request: RequestInput
    headers: HeaderInput
    citations: ReplyInput
    max_results: MaxResultsInput
    cap: MaxResultsInput
    reply: ReplyInput
    error: ReplyInput
    settings: SettingsInput
    price: ReplyInput


JSON: Final = TypeAdapter(JsonValue)
ENVIRONMENT: Final = (PROJECT_ENDPOINT_ENV, MODEL_ENV, CONNECTION_ID_ENV, TOKEN_ENV)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
ARBITRARY: Final[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)


@runtime_checkable
class SearchConfig(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(self, api_base: str | None, optional_params: Params) -> str: ...

    def transform_search_request(
        self, query: str | list[str], optional_params: Params
    ) -> JsonValue: ...

    def transform_search_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        optional_params: Params,
    ) -> SearchResponse: ...


ADAPTER: Final[TypeAdapter[SearchConfig]] = TypeAdapter(SearchConfig, config=ARBITRARY)


class Minter:
    def __init__(self, spec: MinterSpec) -> None:
        self.spec = spec
        self.calls = 0

    def __call__(self) -> str:
        self.calls += 1
        detail = self.spec.get("error")
        if detail is not None:
            raise ValueError(detail)
        return self.spec["token"]


class HiddenCarrier(BaseModel):
    params: Params = Field(alias="_hidden_params")


class ErrorRecord(BaseModel):
    status_code: int


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def logging_object() -> Logging:
    return Logging(
        model="gpt-4.1",
        messages=None,
        stream=False,
        call_type=CallTypes.search,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="azure-search",
        function_id="azure-search",
    )
