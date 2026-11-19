import os
from collections.abc import Callable
from typing import Literal, TypeVar, TypedDict, cast

import litellm
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.ragflow.chat.transformation import RAGFlowConfig
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue

litellm.suppress_debug_info = True
BASE_ENV = "RAGFLOW_API_BASE"
KEY_ENV = "RAGFLOW_API_KEY"
STORED_MODEL = "_ragflow_actual_model"
EMPTY: dict[str, JsonValue] = {}
NO_MESSAGES: list[AllMessageValues] = []
Case = TypeVar("Case")


class Sdk(TypedDict):
    api_base: str | None
    api_key: str | None


class Message(TypedDict):
    role: str
    content: str


class ProviderProbe(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None


class UrlProbe(TypedDict):
    model: str
    api_base: str | None


class HeadersProbe(TypedDict):
    model: str
    api_key: str | None
    headers: dict[str, str]


class RequestProbe(TypedDict):
    model: str
    messages: list[Message]
    options: dict[str, JsonValue]
    stored: str | None


class ProviderInput(TypedDict):
    op: Literal["provider"]
    environment: dict[str, str]
    sdk: Sdk
    cases: list[ProviderProbe]


class UrlInput(TypedDict):
    op: Literal["url"]
    environment: dict[str, str]
    sdk: Sdk
    cases: list[UrlProbe]


class HeadersInput(TypedDict):
    op: Literal["headers"]
    environment: dict[str, str]
    sdk: Sdk
    cases: list[HeadersProbe]


class RequestInput(TypedDict):
    op: Literal["request"]
    environment: dict[str, str]
    sdk: Sdk
    cases: list[RequestProbe]


Input = ProviderInput | UrlInput | HeadersInput | RequestInput


def apply_environment(environment: dict[str, str], sdk: Sdk) -> None:
    os.environ.pop(BASE_ENV, None)
    os.environ.pop(KEY_ENV, None)
    os.environ.update(environment)
    litellm.api_base = sdk["api_base"]
    litellm.api_key = sdk["api_key"]


def error_value(error: ValueError) -> JsonValue:
    class_name = type(error).__name__
    return {"ok": False, "error": {"class": class_name, "message": str(error)}}


def probes(cases: list[Case], probe: Callable[[Case], JsonValue]) -> JsonValue:
    found: list[JsonValue] = []
    for case in cases:
        try:
            found.append(probe(case))
        except ValueError as error:
            found.append(error_value(error))
    return {"probes": found}


def provider_case(case: ProviderProbe) -> JsonValue:
    _, _, key, base = get_llm_provider(
        model=case["model"], api_base=case["api_base"], api_key=case["api_key"]
    )
    return {"ok": True, "base": base, "key": key}


def url_case(case: UrlProbe) -> JsonValue:
    return {
        "ok": True,
        "url": RAGFlowConfig().get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ragflow/chat/transformation.py#L63
            api_base=case["api_base"],
            api_key=None,
            model=case["model"],
            optional_params=EMPTY,
            litellm_params=EMPTY,
        ),
    }


def headers_case(case: HeadersProbe) -> JsonValue:
    params: dict[str, JsonValue] = {}
    headers: dict[str, str] = dict(case["headers"])
    merged: dict[str, JsonValue] = RAGFlowConfig().validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ragflow/chat/transformation.py#L160
        headers=headers,
        model=case["model"],
        messages=NO_MESSAGES,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=EMPTY,
        litellm_params=params,
        api_key=case["api_key"],
    )
    return {"headers": merged, "stored": params.get(STORED_MODEL)}


def request_case(case: RequestProbe) -> JsonValue:
    stored = case["stored"]
    params: dict[str, JsonValue] = {}
    if stored is not None:
        params[STORED_MODEL] = stored
    body: dict[str, JsonValue] = RAGFlowConfig().transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ragflow/chat/transformation.py#L211
        model=case["model"],
        messages=cast("list[AllMessageValues]", case["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=case["options"],
        litellm_params=params,
        headers=EMPTY,
    )
    return {"model": body["model"]}


def provider(input_value: ProviderInput) -> JsonValue:
    return probes(input_value["cases"], provider_case)


def url(input_value: UrlInput) -> JsonValue:
    return probes(input_value["cases"], url_case)


def headers(input_value: HeadersInput) -> JsonValue:
    return probes(input_value["cases"], headers_case)


def request(input_value: RequestInput) -> JsonValue:
    return probes(input_value["cases"], request_case)


def run(input_value: Input) -> JsonValue:
    apply_environment(input_value["environment"], input_value["sdk"])
    if input_value["op"] == "provider":
        return provider(input_value)
    if input_value["op"] == "url":
        return url(input_value)
    if input_value["op"] == "headers":
        return headers(input_value)
    return request(input_value)
