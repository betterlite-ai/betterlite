import logging
from typing import Literal, NoReturn, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.litellm_core_utils.core_helpers import drop_params_env_flag, drop_params_flag
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.responses.utils import ResponsesAPIRequestUtils
from litellm.types.llms.openai import (  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L59
    ResponseInputParam,
)
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams as OptionalParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Argument: TypeAlias = Json | GenericLiteLLMParams | httpx.Response | Logging


class OptionalStep(TypedDict):
    model: str
    supported: list[str] | None
    allowed: list[str]
    global_drop: bool
    call_drop: bool | None
    params: dict[str, Json]
    mapped: dict[str, Json] | None


class OptionalInput(TypedDict):
    action: Literal["optional"]
    steps: list[OptionalStep]


class RequestedStep(TypedDict):
    params: dict[str, Json]
    text_format: dict[str, Json] | None
    text: Json | None


class RequestedInput(TypedDict):
    action: Literal["requested"]
    steps: list[RequestedStep]


class MergedEntry(TypedDict):
    source: int | None
    message: dict[str, Json]


class PromptStep(TypedDict):
    original_input: Json | None
    merged: list[MergedEntry]


class PromptInput(TypedDict):
    action: Literal["prompt"]
    steps: list[PromptStep]


class ForwardedStep(TypedDict):
    extra: dict[str, Json] | None
    client: dict[str, str] | None


class ForwardedInput(TypedDict):
    action: Literal["forwarded_headers"]
    steps: list[ForwardedStep]


class McpStep(TypedDict):
    secret_fields: dict[str, Json] | None
    tools: list[Json] | None


class McpInput(TypedDict):
    action: Literal["mcp_headers"]
    steps: list[McpStep]


class DropFlagsStep(TypedDict):
    value: Json
    environment: dict[str, str]


class DropFlagsInput(TypedDict):
    action: Literal["drop_flags"]
    steps: list[DropFlagsStep]


Input: TypeAlias = (
    OptionalInput | RequestedInput | PromptInput | ForwardedInput | McpInput | DropFlagsInput
)


class Fixture(BaseResponsesAPIConfig):
    def unsupported(self, *_args: Argument, **_kwargs: Argument) -> NoReturn:
        raise NotImplementedError

    def __init__(self, supported: list[str] | None, mapped: dict[str, Json] | None) -> None:
        self.supported_names = supported
        self.mapped_params = mapped
        self.calls: list[dict[str, Json]] = []

    @property
    @override
    def custom_llm_provider(self) -> LlmProviders:
        return LlmProviders.OPENAI

    @override
    def get_supported_openai_params(self, _model: str) -> list[str]:  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L90
        return [] if self.supported_names is None else [*self.supported_names]

    @override
    def map_openai_params(
        self, response_api_optional_params: OptionalParams, model: str, drop_params: bool
    ) -> dict[str, Json]:
        received = cast("dict[str, Json]", dict(response_api_optional_params))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1277
        self.calls.append({"model": model, "drop_params": drop_params, "params": received})
        return received if self.mapped_params is None else dict(self.mapped_params)

    @override
    def transform_responses_api_request(  # ast-grep-ignore: no-long-names
        self, model: str, input: str | ResponseInputParam,
        response_api_optional_request_params: dict[str, Json],  # ast-grep-ignore: no-long-names
        litellm_params: GenericLiteLLMParams, headers: dict[str, Json],
    ) -> dict[str, Json]:
        raise NotImplementedError

    @override
    def transform_list_input_items_request(  # ast-grep-ignore: no-long-names
        self, response_id: str, api_base: str, litellm_params: GenericLiteLLMParams,
        headers: dict[str, Json], after: str | None = None, before: str | None = None,
        include: list[str] | None = None, limit: int = 20, order: Literal["asc", "desc"] = "desc",
    ) -> tuple[str, dict[str, Json]]:
        raise NotImplementedError

    @override
    def transform_compact_response_api_request(  # ast-grep-ignore: no-long-names
        self, model: str, input: str | ResponseInputParam,
        response_api_optional_request_params: dict[str, Json],  # ast-grep-ignore: no-long-names
        api_base: str, litellm_params: GenericLiteLLMParams, headers: dict[str, Json],
    ) -> tuple[str, dict[str, Json]]:
        raise NotImplementedError

    get_complete_url = unsupported
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


class WarningCollector(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def optional_step(step: OptionalStep) -> Json:
    params: dict[str, Json] = dict(step["params"])
    litellm.drop_params = step["global_drop"]
    fixture = Fixture(step["supported"], step["mapped"])
    try:
        mapped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L205
            "dict[str, Json]",
            ResponsesAPIRequestUtils.get_optional_params_responses_api(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L205
                model=step["model"],
                responses_api_provider_config=fixture,
                response_api_optional_params=cast("OptionalParams", params),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1277
                allowed_openai_params=step["allowed"],
                drop_params=step["call_drop"],
            ),
        )
        status: int | None = None
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        mapped = {}
        status = error.status_code
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L205
        "Json",
        {"ok": status is None, "status": status, "value": mapped, "calls": fixture.calls,
         "params_after": params},
    )


def requested_step(step: RequestedStep) -> Json:
    params: dict[str, Json] = dict(step["params"])
    optional = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L259
        "Json",
        ResponsesAPIRequestUtils.get_requested_response_api_optional_param(params),
    )
    try:
        text = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L1039
            "Json | None",
            ResponsesAPIRequestUtils.convert_text_format_to_text_param(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L1039
                text_format=step["text_format"],
                text=step["text"],
            ),
        )
        failed = False
    except KeyError:
        text, failed = None, True
    return {"optional": optional, "remaining": params, "text": text, "text_failed": failed}


def prompt_step(step: PromptStep) -> Json:
    client = ResponsesAPIRequestUtils.responses_input_to_chat_messages(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L94
        cast("str | ResponseInputParam", step["original_input"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L94
    )
    if not step["merged"]:
        return cast("Json", {"client": client, "input": None, "warnings": []})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L94
    merged = [
        entry["message"] if entry["source"] is None else client[entry["source"]]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L94
        for entry in step["merged"]
    ]
    collector = WarningCollector()
    verbose_logger = logging.getLogger("LiteLLM")
    previous_level = verbose_logger.level
    verbose_logger.addHandler(collector)
    verbose_logger.setLevel(logging.WARNING)
    try:
        result = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L106
            "Json",
            ResponsesAPIRequestUtils.merge_prompt_management_input(
                original_input=cast("str | ResponseInputParam", step["original_input"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L106
                client_input=client,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L106
                merged_input=merged,  # type: ignore[misc, arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L106
            ),
        )
    finally:
        verbose_logger.removeHandler(collector)
        verbose_logger.setLevel(previous_level)
    return cast("Json", {"client": client, "input": result, "warnings": collector.messages})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L94


def forwarded_step(step: ForwardedStep) -> Json:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L157
        "Json",
        ResponsesAPIRequestUtils.merge_client_forwarded_headers(step["extra"], step["client"]),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L157
    )


def mcp_step(step: McpStep) -> Json:
    auth, servers, oauth, raw = ResponsesAPIRequestUtils.extract_mcp_headers_from_request(
        secret_fields=step["secret_fields"],
        tools=step["tools"],
    )
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L1073
        "Json",
        {"auth_header": auth, "server_auth_headers": servers, "oauth2_headers": oauth,
         "raw_headers": raw},
    )


def drop_flags_step(step: DropFlagsStep) -> Json:
    logger = logging.getLogger("LiteLLM")
    value = step["value"]
    enabled = (
        drop_params_env_flag(step["environment"], logger)
        if value is None
        else drop_params_flag(value, "litellm_settings.drop_params", logger)
    )
    return {"drop_params": enabled}


def run(input_value: Input) -> Json:
    match input_value["action"]:
        case "optional":
            return [optional_step(step) for step in input_value["steps"]]
        case "requested":
            return [requested_step(step) for step in input_value["steps"]]
        case "prompt":
            return [prompt_step(step) for step in input_value["steps"]]
        case "forwarded_headers":
            return [forwarded_step(step) for step in input_value["steps"]]
        case "mcp_headers":
            return [mcp_step(step) for step in input_value["steps"]]
        case "drop_flags":
            return [drop_flags_step(step) for step in input_value["steps"]]
