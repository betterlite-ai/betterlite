from datetime import UTC, datetime
from typing import Final, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.common_utils import AzureOpenAIError
from openai import (
    APIError,
    APIResponseValidationError,
    APIStatusError,
    APITimeoutError,
    OpenAIError,
)
from pydantic import JsonValue, ValidationError
from typing_extensions import override

from .types import (
    BODY,
    CALL_ARGS,
    KINDS,
    MODEL,
    ORIGINAL,
    PARAMS_VIEW,
    REPLY,
    TIMEOUT_DIAGNOSTIC,
    AnyClient,
    CallArgs,
    HiddenResponse,
    JsonDump,
    ReplyView,
    UsageView,
)


def args_view(additional: CallArgs) -> dict[str, JsonValue]:
    return {
        "headers": additional.headers,
        "api_base": additional.api_base,
        "api_version": additional.api_version,
        "acompletion": additional.acompletion,
        "complete_input_dict": additional.complete_input_dict,
        "openai_sdk": additional.openai_sdk,
    }


def pre_view(params: dict[str, JsonValue]) -> JsonValue:
    view = args_view(CALL_ARGS.validate_python(params.get("additional_args")))
    view["input"] = params.get("input")
    view["api_key"] = params.get("api_key")
    return view


def original_id(params: dict[str, JsonValue]) -> JsonValue:
    try:
        original = ORIGINAL.validate_python(params.get("original_response"))
    except ValidationError:
        return None
    return original.get("id")


def post_view(params: dict[str, JsonValue]) -> JsonValue:
    view = args_view(CALL_ARGS.validate_python(params.get("additional_args")))
    view["input"] = params.get("input")
    view["original_id"] = original_id(params)
    return view


class Recorder(Logging):
    pre: list[JsonValue]
    post: list[JsonValue]

    def __init__(self) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
            model=MODEL,
            messages=None,
            stream=False,
            call_type="completion",
            start_time=datetime(2024, 1, 2, tzinfo=UTC),
            litellm_call_id="call",
            function_id="function",
        )
        self.pre = []
        self.post = []

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.pre.append(pre_view(params))

    @override
    def post_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.post.append(post_view(params))


def stored_headers(recorder: Recorder) -> JsonValue:
    details: dict[str, JsonValue] = recorder.model_call_details
    headers = details.get("response_headers")
    if headers is None:
        return None
    return BODY.validate_python(headers)


def choice_view(reply: ReplyView) -> JsonValue:
    choices = reply.get("choices")
    if not choices:
        return None
    message = choices[0].get("message")
    if message is None:
        return None
    return {
        "index": choices[0].get("index"),
        "finish_reason": choices[0].get("finish_reason"),
        "role": message.get("role"),
        "content": message.get("content"),
        "has_tool_calls": message.get("tool_calls") is not None,
    }


def usage_view(usage: UsageView | None) -> JsonValue:
    if usage is None:
        return {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }


def hidden_view(result: JsonDump) -> JsonValue:
    hidden = cast(HiddenResponse, result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L742-L745
    params = hidden._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L742-L745
    additional = BODY.validate_python(params.get("additional_headers"))
    raw = params.get("headers")
    if raw is None:
        return {"additional_headers": additional}
    return {"additional_headers": additional, "headers": BODY.validate_python(raw)}


def reply_view(result: JsonDump) -> JsonValue:
    reply = REPLY.validate_json(result.model_dump_json())
    return {
        "id": reply.get("id"),
        "model": reply.get("model"),
        "created": reply.get("created"),
        "choice": choice_view(reply),
        "usage": usage_view(reply.get("usage")),
        "hidden": hidden_view(result),
    }


def error_fields(
    status: int | None,
    headers: httpx.Headers | dict[str, str] | None,
    message: str,
    *,
    has_body: bool,
) -> JsonValue:
    retry: JsonValue = None
    if headers is not None:
        retry = BODY.validate_python(headers).get("retry-after")
    return {
        "status": status,
        "retry_after": retry,
        "has_headers": headers is not None,
        "has_body": has_body,
        "has_timeout_diagnostic": TIMEOUT_DIAGNOSTIC in message,
    }


def azure_error_view(error: AzureOpenAIError) -> JsonValue:
    view = cast(ErrorView, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return error_fields(
        view.status_code,
        view.headers,
        view.message,
        has_body=view.body is not None,
    )


def sdk_error_view(error: OpenAIError) -> JsonValue:
    match error:
        case APITimeoutError():
            return error_fields(500, None, error.message, has_body=False)
        case APIStatusError():
            return error_fields(
                error.status_code,
                error.response.headers,
                error.message,
                has_body=error.body is not None,
            )
        case APIResponseValidationError():
            return error_fields(
                error.status_code,
                error.response.headers,
                error.message,
                has_body=error.body is not None,
            )
        case APIError():
            return error_fields(500, None, error.message, has_body=error.body is not None)
        case OpenAIError():
            return error_fields(500, None, str(error), has_body=False)


def kind_name(client: AnyClient) -> str:
    kind = KINDS.get(type(client).__name__)
    if kind is None:
        return "other"
    return kind


def client_view(client: AnyClient | None) -> JsonValue:
    if client is None:
        return None
    return {"kind": kind_name(client), "api_key": client.api_key}


class ParamsView(TypedDict):
    azure_ad_token: JsonValue
    max_retries: JsonValue
    timeout: JsonValue


def params_view(params: JsonValue) -> JsonValue:
    if params is None:
        return None
    view = PARAMS_VIEW.validate_python(params)
    return ParamsView(
        azure_ad_token=view.get("azure_ad_token"),
        max_retries=view.get("max_retries"),
        timeout=view.get("timeout"),
    )


FORWARDED: Final = (
    "model",
    "api_base",
    "api_version",
    "api_key",
    "azure_ad_token",
    "timeout",
    "acompletion",
    "organization",
    "drop_params",
    "shared_session",
    "dynamic_params",
    "headers",
    "optional_params",
)


def forward_view(params: dict[str, JsonValue]) -> JsonValue:
    client = cast(AnyClient | None, params.get("client"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L645-L646
    view = {key: params.get(key) for key in FORWARDED}
    view["litellm_params"] = params_view(params.get("litellm_params"))
    view["client"] = client_view(client)
    return view
