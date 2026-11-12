import asyncio
from typing import Literal, Protocol, TypeAlias, TypedDict, TypeGuard, cast

from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CapabilitiesInput(TypedDict):
    action: Literal["base_capabilities"]
    model: str | None
    stream: bool | None
    custom_llm_provider: str | None


class SigningInput(TypedDict):
    action: Literal["signing_passthrough"]
    model: str
    headers: dict[str, Json]
    optional_params: dict[str, Json]
    request_data: dict[str, Json]
    api_base: str
    api_key: str | None
    stream: bool | None
    fake_stream: bool | None
    extra_body: dict[str, Json]
    request: dict[str, Json]


class WebsocketInput(TypedDict):
    action: Literal["websocket_url"]
    api_bases: list[str | None]


class NormalizationInput(TypedDict):
    action: Literal["request_normalization"]
    steps: list[dict[str, Json]]


class ForwardingInput(TypedDict):
    action: Literal["async_request_forwarding"]
    model: str
    input: str
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    headers: dict[str, Json]


class ErrorInput(TypedDict):
    action: Literal["base_error_fields"]
    message: str
    status_code: int
    headers: dict[str, Json]


Input: TypeAlias = (
    CapabilitiesInput | SigningInput | WebsocketInput | NormalizationInput
    | ForwardingInput | ErrorInput
)


class FixtureState(Protocol):
    forwarded: list[Json]
    complete_url_calls: int


ABSTRACT_MEMBERS: tuple[str, ...] = (
    "get_supported_openai_params", "map_openai_params", "validate_environment",
    "transform_response_api_response", "transform_streaming_response",
    "transform_delete_response_api_request", "transform_delete_response_api_response",
    "transform_get_response_api_request", "transform_get_response_api_response",
    "transform_list_input_items_response", "transform_cancel_response_api_request",
    "transform_cancel_response_api_response", "transform_compact_response_api_request",
    "transform_compact_response_api_response",
)


def init(self: BaseResponsesAPIConfig) -> None:
    state = cast("FixtureState", self)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L30
    state.forwarded = []
    state.complete_url_calls = 0


def provider(self: BaseResponsesAPIConfig) -> LlmProviders:
    del self
    return LlmProviders.OPENAI


def unsupported(self: BaseResponsesAPIConfig, **kwargs: Json) -> Json:
    del self, kwargs
    raise NotImplementedError


def complete_url(
    self: BaseResponsesAPIConfig, api_base: str | None, litellm_params: dict[str, Json]
) -> str:
    state = cast("FixtureState", self)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L30
    state.complete_url_calls += 1
    return BaseResponsesAPIConfig.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L107
        self, api_base=api_base, litellm_params=litellm_params
    )


def list_input_items(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L220
    self: BaseResponsesAPIConfig,
    response_id: str,
    api_base: str,
    litellm_params: GenericLiteLLMParams,
    headers: dict[str, Json],
    after: str | None = None, before: str | None = None,
    include: list[str] | None = None, limit: int = 20,
    order: Literal["asc", "desc"] = "desc",
) -> tuple[str, dict[str, Json]]:
    del self, litellm_params, headers
    include_json: Json = None if include is None else [*include]
    items: dict[str, Json] = {
        "after": after, "before": before, "include": include_json, "limit": limit, "order": order,
    }
    return f"{api_base}/{response_id}/input_items", items


def capture(
    self: BaseResponsesAPIConfig, litellm_params: GenericLiteLLMParams, **kwargs: Json
) -> Json:
    supplied: Json = litellm_params.model_dump(exclude_none=True, exclude_unset=True)
    call: Json = {**kwargs, "litellm_params": supplied}
    cast("FixtureState", self).forwarded.append(call)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L30
    return {"transformed": kwargs["model"]}


FIXTURE_MEMBERS = {"__init__": init, "custom_llm_provider": property(provider),
    "get_complete_url": complete_url,
    "transform_list_input_items_request": list_input_items,
    "transform_responses_api_request": capture,
    **dict.fromkeys(ABSTRACT_MEMBERS, unsupported),
}


FIXTURE: type[BaseResponsesAPIConfig] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L30
    "type[BaseResponsesAPIConfig]",
    type("Fixture", (BaseResponsesAPIConfig,), FIXTURE_MEMBERS),
)


def is_capabilities(fields: Input) -> TypeGuard[CapabilitiesInput]:
    action = fields["action"]
    return action == "base_capabilities"


def is_signing(fields: Input) -> TypeGuard[SigningInput]:
    action = fields["action"]
    return action == "signing_passthrough"


def is_websocket(fields: Input) -> TypeGuard[WebsocketInput]:
    action = fields["action"]
    return action == "websocket_url"


def is_normalization(fields: Input) -> TypeGuard[NormalizationInput]:
    action = fields["action"]
    return action == "request_normalization"


def is_forwarding(fields: Input) -> TypeGuard[ForwardingInput]:
    action = fields["action"]
    return action == "async_request_forwarding"


def is_error_fields(fields: Input) -> TypeGuard[ErrorInput]:
    action = fields["action"]
    return action == "base_error_fields"


def capabilities_report(fields: CapabilitiesInput) -> Json:
    config = FIXTURE()
    input_items = list_input_items(
        config, "resp_a", "https://example.test", GenericLiteLLMParams(), {}
    )
    return {
        "provider": config.custom_llm_provider.value,
        "native_file_search": config.supports_native_file_search(),
        "encrypted_agent_messages": config.supports_encrypted_agent_messages(),
        "native_websocket": config.supports_native_websocket(),
        "model_in_websocket_url": config.model_in_websocket_url(),
        "should_fake_stream": config.should_fake_stream(
            model=fields["model"],
            stream=fields["stream"],
            custom_llm_provider=fields["custom_llm_provider"],
        ),
        "input_items": input_items[1],
    }


def signing_report(fields: SigningInput) -> Json:
    config = FIXTURE()
    signed: tuple[Json, bytes | None] = config.sign_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L69
        headers=fields["headers"], optional_params=fields["optional_params"],
        request_data=fields["request_data"], api_base=fields["api_base"],
        api_key=fields["api_key"], model=fields["model"],
        stream=fields["stream"], fake_stream=fields["fake_stream"],
    )
    headers, body = signed  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L69
    extra_body = config.transform_extra_body(
        fields["extra_body"], fields["request"], fields["model"], GenericLiteLLMParams()
    )
    environment: Json = BaseResponsesAPIConfig.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L103
        config, headers=fields["headers"], model=fields["model"], litellm_params=None
    )
    return {
        "headers": headers,
        "signed_body": None if body is None else [*body],
        "extra_body": cast("dict[str, Json]", dict(extra_body)),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L376
        "environment": environment,
    }


def url_outcome(config: BaseResponsesAPIConfig, api_base: str | None, *, websocket: bool) -> Json:
    litellm_params: dict[str, Json] = {}
    try:
        if websocket:
            return config.get_websocket_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L276
                api_base=api_base, litellm_params=litellm_params
            )
        return BaseResponsesAPIConfig.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L107
            config, api_base=api_base, litellm_params=litellm_params
        )
    except ValueError as error:
        return {"error": {"class": type(error).__name__, "message": str(error)}}


def websocket_step(config: BaseResponsesAPIConfig, api_base: str | None) -> Json:
    complete = url_outcome(config, api_base, websocket=False)
    websocket = url_outcome(config, api_base, websocket=True)
    calls = cast("FixtureState", config).complete_url_calls  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L30
    return {"complete_url": complete, "websocket_url": websocket, "websocket_calls": calls}


def websocket_report(fields: WebsocketInput) -> Json:
    config = FIXTURE()
    return [websocket_step(config, api_base) for api_base in fields["api_bases"]]


def normalization_report(fields: NormalizationInput) -> Json:
    return [
        BaseResponsesAPIConfig.normalize_responses_api_request_dict(step)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L378
        for step in fields["steps"]
    ]


def forwarding_report(fields: ForwardingInput) -> Json:
    config = FIXTURE()
    result: Json = asyncio.run(  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L134
        config.async_transform_responses_api_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L134
            model=fields["model"],
            input=fields["input"],
            response_api_optional_request_params=fields["optional_params"],
            litellm_params=GenericLiteLLMParams.model_validate(fields["litellm_params"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/router.py#L381
            headers=fields["headers"],
        )
    )
    forwarded = cast("FixtureState", config).forwarded  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L30
    return {"calls": forwarded, "result": result}


def error_report(fields: ErrorInput) -> Json:
    config = FIXTURE()
    try:
        config.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L246
            error_message=fields["message"],
            status_code=fields["status_code"],
            headers=fields["headers"],
        )
    except BaseLLMException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41
        headers: Json = cast("Json", raised.headers)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L46
        return {
            "raised": True, "status_code": raised.status_code,
            "message": raised.message, "headers": headers,
        }
    raise ValueError("the base error class returned without raising")


def run(input_value: Input) -> Json:
    if is_capabilities(input_value):
        return capabilities_report(input_value)
    if is_signing(input_value):
        return signing_report(input_value)
    if is_websocket(input_value):
        return websocket_report(input_value)
    if is_normalization(input_value):
        return normalization_report(input_value)
    if is_forwarding(input_value):
        return forwarding_report(input_value)
    if is_error_fields(input_value):
        return error_report(input_value)
    raise ValueError("unsupported action")
