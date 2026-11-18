import os
from datetime import UTC, datetime
from typing import Literal, TypedDict

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
import litellm
from litellm.llms.sagemaker.chat.transformation import SagemakerChatConfig
from pydantic import JsonValue, TypeAdapter

CALLER_KEY = "sagemaker-caller-key"


class UrlInput(TypedDict):
    function: Literal["url"]
    model: str
    region: str
    base_url: str | None
    stream: bool


class ComponentInput(TypedDict):
    function: Literal["component_header"]
    model_id: str | None
    headers: dict[str, str]


class CredentialInput(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: str | None


class SignedInput(TypedDict):
    function: Literal["signed_request"]
    api_base: str
    model: str
    headers: dict[str, str]
    optional_params: dict[str, JsonValue]
    request_data: dict[str, JsonValue]
    credentials: CredentialInput
    now: int


Input = UrlInput | ComponentInput | SignedInput
INPUT = TypeAdapter(Input)
CONFIG = SagemakerChatConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L42-L44

litellm.suppress_debug_info = True


class FrozenClock:
    def __init__(self, frozen: datetime) -> None:
        self.frozen = frozen

    def __call__(self) -> datetime:
        return self.frozen


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = INPUT.validate_python(input_value)
    if case["function"] == "url":
        return url_value(case)
    if case["function"] == "component_header":
        return component_value(case)
    return signed_value(case)


def url_value(case: UrlInput) -> JsonValue:
    optional_params: dict[str, JsonValue] = {"aws_region_name": case["region"]}
    if case["base_url"] is not None:
        optional_params["sagemaker_base_url"] = case["base_url"]
    litellm_params: dict[str, JsonValue] = {}
    url = CONFIG.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L84-L92
        None,
        None,
        case["model"],
        optional_params,
        litellm_params,
        case["stream"],
    )
    return {"url": url}


def component_value(case: ComponentInput) -> JsonValue:
    optional_params: dict[str, JsonValue] = {}
    if case["model_id"] is not None:
        optional_params["model_id"] = case["model_id"]
    litellm_params: dict[str, JsonValue] = {}
    headers: dict[str, JsonValue] = CONFIG.validate_environment(  # pyright: ignore[reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L49-L58
        case["headers"],
        "model",
        [],  # type: ignore[misc]  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L53
        optional_params,
        litellm_params,
    )
    return {"headers": headers}


def signed_value(case: SignedInput) -> JsonValue:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull
    frozen = datetime.fromtimestamp(case["now"], tz=UTC).replace(tzinfo=None)
    botocore.auth.get_current_datetime = FrozenClock(frozen)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
    keys = case["credentials"]
    optional_params: dict[str, JsonValue] = dict(case["optional_params"])
    optional_params["aws_access_key_id"] = keys["access_key_id"]
    optional_params["aws_secret_access_key"] = keys["secret_access_key"]
    if keys["session_token"] is not None:
        optional_params["aws_session_token"] = keys["session_token"]
    signed_headers: dict[str, JsonValue]
    signed_body: bytes | None
    signed_headers, signed_body = CONFIG.sign_request(  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L110-L130
        headers=case["headers"],
        optional_params=optional_params,
        request_data=case["request_data"],
        api_base=case["api_base"],
        api_key=CALLER_KEY,
        model=case["model"],
        stream=False,
        fake_stream=False,
    )
    body_text = "" if signed_body is None else signed_body.decode()
    return {
        "ok": True,
        "method": "POST",
        "url": case["api_base"],
        "headers": signed_headers,
        "body": body_text,
    }
