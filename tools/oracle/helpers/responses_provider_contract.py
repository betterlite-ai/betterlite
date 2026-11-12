import asyncio
from typing import Literal, NoReturn, TypeAlias, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.types.llms.openai import (  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L59
    ResponseInputParam,
)
from litellm.types.llms.openai import (
    ResponsesAPIOptionalRequestParams as OptionalParams,
)
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from typing_extensions import override

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


Argument: TypeAlias = Json | GenericLiteLLMParams | httpx.Response | Logging
Input: TypeAlias = (
    CapabilitiesInput | SigningInput | WebsocketInput | NormalizationInput | ForwardingInput
    | ErrorInput
)


class Fixture(BaseResponsesAPIConfig):
    def unsupported(self, *_args: Argument, **_kwargs: Argument) -> NoReturn:
        raise NotImplementedError

    def __init__(self) -> None:
        self.forwarded: list[Json] = []
        self.complete_url_calls = 0

    @property
    @override
    def custom_llm_provider(self) -> LlmProviders:
        return LlmProviders.OPENAI

    @override
    def get_complete_url(self, api_base: str | None, litellm_params: dict[str, Json]) -> str:
        self.complete_url_calls += 1
        return BaseResponsesAPIConfig.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L107
            self, api_base=api_base, litellm_params=litellm_params
        )

    @override
    def transform_responses_api_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        input: str | ResponseInputParam,
        response_api_optional_request_params: dict[str, Json],  # ast-grep-ignore: no-long-names
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, Json],
    ) -> dict[str, Json]:
        supplied: Json = litellm_params.model_dump(exclude_none=True, exclude_unset=True)
        call: Json = {
            # ast-grep-ignore: no-long-names
            "response_api_optional_request_params": response_api_optional_request_params,
            "model": model,
            "input": cast("Json", input),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L127
            "litellm_params": supplied,
            "headers": headers,
        }
        self.forwarded.append(call)
        return {"transformed": model}

    @override
    def transform_list_input_items_request(  # ast-grep-ignore: no-long-names
        self,
        response_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, Json],
        after: str | None = None,
        before: str | None = None,
        include: list[str] | None = None,
        limit: int = 20,
        order: Literal["asc", "desc"] = "desc",
    ) -> tuple[str, dict[str, Json]]:
        query: dict[str, Json] = {"after": after, "before": before, "limit": limit, "order": order}
        query["include"] = None if include is None else [*include]
        return f"{api_base}/{response_id}/input_items", query

    @override
    def transform_compact_response_api_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        input: str | ResponseInputParam,
        response_api_optional_request_params: dict[str, Json],  # ast-grep-ignore: no-long-names
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, Json],
    ) -> tuple[str, dict[str, Json]]:
        raise NotImplementedError

    @override
    def map_openai_params(
        self, response_api_optional_params: OptionalParams, model: str, drop_params: bool
    ) -> dict[str, Json]:
        raise NotImplementedError

    get_supported_openai_params = unsupported
    validate_environment = unsupported
    transform_response_api_response = unsupported  # ast-grep-ignore: no-long-names
    transform_streaming_response = unsupported
    transform_delete_response_api_request = unsupported  # ast-grep-ignore: no-long-names
    transform_delete_response_api_response = unsupported  # ast-grep-ignore: no-long-names
    transform_get_response_api_request = unsupported  # ast-grep-ignore: no-long-names
    transform_get_response_api_response = unsupported  # ast-grep-ignore: no-long-names
    transform_list_input_items_response = unsupported  # ast-grep-ignore: no-long-names
    transform_cancel_response_api_request = unsupported  # ast-grep-ignore: no-long-names
    transform_cancel_response_api_response = unsupported  # ast-grep-ignore: no-long-names
    transform_compact_response_api_response = unsupported  # ast-grep-ignore: no-long-names


def capabilities_report(fields: CapabilitiesInput) -> Json:
    config = Fixture()
    input_items = config.transform_list_input_items_request(
        "resp_a", "https://example.test", GenericLiteLLMParams(), {}
    )[1]
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
        "input_items": input_items,
    }


def signing_report(fields: SigningInput) -> Json:
    config = Fixture()
    signed: tuple[Json, bytes | None] = config.sign_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L69
        headers=fields["headers"],
        optional_params=fields["optional_params"],
        request_data=fields["request_data"],
        api_base=fields["api_base"],
        api_key=fields["api_key"],
        model=fields["model"],
        stream=fields["stream"],
        fake_stream=fields["fake_stream"],
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


def websocket_report(fields: WebsocketInput) -> Json:
    config = Fixture()
    steps: list[Json] = []
    for api_base in fields["api_bases"]:
        complete = url_outcome(config, api_base, websocket=False)
        websocket = url_outcome(config, api_base, websocket=True)
        steps.append(
            {
                "complete_url": complete,
                "websocket_url": websocket,
                "websocket_calls": config.complete_url_calls,
            }
        )
    return steps


def normalization_report(fields: NormalizationInput) -> Json:
    return [
        BaseResponsesAPIConfig.normalize_responses_api_request_dict(step)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L378
        for step in fields["steps"]
    ]


def forwarding_report(fields: ForwardingInput) -> Json:
    config = Fixture()
    result: Json = asyncio.run(  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L134
        config.async_transform_responses_api_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L134
            model=fields["model"],
            input=fields["input"],
            response_api_optional_request_params=fields["optional_params"],
            litellm_params=GenericLiteLLMParams.model_validate(fields["litellm_params"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/router.py#L381
            headers=fields["headers"],
        )
    )
    return {"calls": config.forwarded, "result": result}


def error_report(fields: ErrorInput) -> Json:
    config = Fixture()
    try:
        config.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L246
            error_message=fields["message"],
            status_code=fields["status_code"],
            headers=fields["headers"],
        )
    except BaseLLMException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41
        headers: Json = cast("Json", raised.headers)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L46
        return {
            "raised": True,
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": headers,
        }
    raise ValueError("the base error class returned without raising")


def run(input_value: Input) -> Json:
    match input_value["action"]:
        case "base_capabilities":
            return capabilities_report(input_value)
        case "signing_passthrough":
            return signing_report(input_value)
        case "websocket_url":
            return websocket_report(input_value)
        case "request_normalization":
            return normalization_report(input_value)
        case "async_request_forwarding":
            return forwarding_report(input_value)
        case "base_error_fields":
            return error_report(input_value)
