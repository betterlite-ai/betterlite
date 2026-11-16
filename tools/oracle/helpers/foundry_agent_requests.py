import os
from collections.abc import Callable
from typing import Final, Literal, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.azure_ai.agents.handler import AzureAIAgentsHandler as AgentHandler
from litellm.llms.azure_ai.agents.transformation import AzureAIAgentsConfig, AzureAIAgentsError
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

JsonObject: TypeAlias = dict[str, JsonValue]
EMPTY: Final[JsonObject] = {}
NO_MESSAGES: Final[list[JsonObject]] = []


class AgentIdCall(TypedDict):
    model: str
    optional_params: JsonObject


class ParamsCall(TypedDict):
    model: str
    non_default_params: JsonObject
    optional_params: JsonObject


class ProviderInfoCall(TypedDict):
    api_base: str | None
    api_key: str | None
    environment: dict[str, str]


class UrlCall(TypedDict):
    api_base: str | None


class RequestCall(TypedDict):
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject


class PreparationCall(TypedDict):
    model: str
    api_base: str
    api_key: str
    optional_params: JsonObject
    headers: dict[str, str]


class ValidationCall(TypedDict):
    headers: dict[str, str]
    api_key: str


class CheckCall(TypedDict):
    status: int
    body: str
    expected: list[int]
    error_msg: str


class ErrorCall(TypedDict):
    status: int
    message: str
    headers: dict[str, str]


class PathCall(TypedDict):
    function: Literal[
        "threads", "messages", "list_messages", "runs", "run_status", "thread_and_run"
    ]
    api_base: str
    api_version: str
    thread_id: str
    run_id: str


class FlagCall(TypedDict):
    model: str
    stream: bool


class Input(TypedDict, total=False):
    agent_ids: list[AgentIdCall]
    params: list[ParamsCall]
    provider_infos: list[ProviderInfoCall]
    complete_urls: list[UrlCall]
    requests: list[RequestCall]
    preparations: list[PreparationCall]
    validations: list[ValidationCall]
    checks: list[CheckCall]
    errors: list[ErrorCall]
    paths: list[PathCall]
    flags: list[FlagCall]


def agent_id_answer(config: AzureAIAgentsConfig, call: AgentIdCall) -> JsonValue:
    selected = config._get_agent_id(call["model"], call["optional_params"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L174-L185
    return {"ok": True, "value": selected}


def params_answer(config: AzureAIAgentsConfig, call: ParamsCall) -> JsonValue:
    mapped = cast("JsonObject", config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L128-L138
        call["non_default_params"], call["optional_params"], model=call["model"], drop_params=False
    ))
    supported = list[JsonValue](config.get_supported_openai_params(call["model"]))
    return {"supported": supported, "mapped": mapped}


def provider_answer(config: AzureAIAgentsConfig, call: ProviderInfoCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["environment"])
    base, key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L104-L120
        call["api_base"], call["api_key"]
    )
    return {"ok": True, "value": {"api_base": base, "api_key": key}}


def complete_url_answer(config: AzureAIAgentsConfig, call: UrlCall) -> JsonValue:
    try:
        url = config.get_complete_url(call["api_base"], None, "model", EMPTY, EMPTY)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L144-L172
    except ValueError:
        return {"ok": False, "error": {"status": None}}
    return {"ok": True, "value": url}


def request_answer(config: AzureAIAgentsConfig, call: RequestCall) -> JsonValue:
    body = cast("JsonObject", config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L187-L234
        model=call["model"], messages=cast("list[AllMessageValues]", call["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=call["optional_params"], litellm_params=EMPTY, headers=EMPTY,
    ))
    return {"ok": True, "value": body}


def validation_answer(config: AzureAIAgentsConfig, call: ValidationCall) -> JsonValue:
    validated = cast("JsonObject", config.validate_environment(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L236-L261
        headers=call["headers"], model="model", optional_params=EMPTY,
        litellm_params=EMPTY, api_key=call["api_key"],
        messages=cast("list[AllMessageValues]", NO_MESSAGES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    ))
    return {"ok": True, "value": {"headers": validated}}


def preparation_answer(handler: AgentHandler, call: PreparationCall) -> JsonValue:
    headers, api_version, agent_id, thread_id, api_base = handler._prepare_completion_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L258-L289
        call["model"],
        call["api_base"],
        call["api_key"],
        call["optional_params"],
        call["headers"],
    )
    return {
        "ok": True,
        "value": {
            "headers": cast("JsonObject", headers),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L258-L289
            "api_version": str(api_version),
            "agent_id": agent_id,
            "thread_id": thread_id,
            "api_base": api_base,
        },
    }


def check_answer(handler: AgentHandler, call: CheckCall) -> JsonValue:
    response = httpx.Response(
        call["status"], text=call["body"], request=httpx.Request("POST", "https://host")
    )
    check = cast(Callable[[httpx.Response, list[int], str], None], handler._check_response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L291-L297
    try:
        check(response, call["expected"], call["error_msg"])
    except AzureAIAgentsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L48-L49
        return {"ok": False, "error": {"status": error.status_code}}
    return {"ok": True}


def error_answer(config: AzureAIAgentsConfig, call: ErrorCall) -> JsonValue:
    error = config.get_error_class(call["message"], call["status"], call["headers"])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L263-L264
    return {
        "status": error.status_code,
        "headers_dropped": cast("object | None", error.headers) is None,  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L263-L264
    }


def path_url(handler: AgentHandler, call: PathCall) -> str:
    base = call["api_base"]
    version = call["api_version"]
    thread = call["thread_id"]
    match call["function"]:
        case "threads":
            return handler._build_thread_url(base, version)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L131-L132
        case "thread_and_run":
            return handler._build_create_thread_and_run_url(base, version)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L151-L153
        case "run_status":
            return handler._build_run_status_url(base, thread, call["run_id"], version)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L142-L145
        case "runs":
            return handler._build_runs_url(base, thread, version)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L138-L140
        case "list_messages":
            return handler._build_list_messages_url(base, thread, version)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L147-L149
        case "messages":
            return handler._build_messages_url(base, thread, version)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L134-L136


def path_answer(handler: AgentHandler, call: PathCall) -> JsonValue:
    try:
        url = path_url(handler, call)
    except ValueError:
        return {"ok": False, "error": {"status": None}}
    return {"ok": True, "value": url}


def flag_answer(config: AzureAIAgentsConfig, call: FlagCall) -> JsonValue:
    response = httpx.Response(200, request=httpx.Request("POST", "https://host"))
    model_response = ModelResponse()
    transformed = config.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L289-L307
        model=call["model"],
        raw_response=response,
        model_response=model_response,
        logging_obj=None,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L294
        request_data=EMPTY,
        messages=cast("list[AllMessageValues]", NO_MESSAGES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=EMPTY,
        litellm_params=EMPTY,
        encoding=None,
    )
    return {
        "model": call["model"],
        "stream": call["stream"],
        "should_fake_stream": config.should_fake_stream(call["model"], call["stream"]),
        "has_custom_stream_wrapper": config.has_custom_stream_wrapper,
        "supports_stream_param_in_request_body": config.supports_stream_param_in_request_body,
        "transform_response_is_noop": transformed is model_response,
    }


def config_reports(argument: Input) -> JsonValue:
    config = AzureAIAgentsConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L77-L78
    if "agent_ids" in argument:
        return {"calls": [agent_id_answer(config, call) for call in argument["agent_ids"]]}
    if "params" in argument:
        return {"calls": [params_answer(config, call) for call in argument["params"]]}
    if "requests" in argument:
        return {"calls": [request_answer(config, call) for call in argument["requests"]]}
    if "validations" in argument:
        return {"calls": [validation_answer(config, call) for call in argument["validations"]]}
    if "errors" in argument:
        return {"calls": [error_answer(config, call) for call in argument["errors"]]}
    if "flags" in argument:
        return {"calls": [flag_answer(config, call) for call in argument["flags"]]}
    raise ValueError("expected a foundry agent request variant")


def provider_reports(argument: Input) -> JsonValue:
    config = AzureAIAgentsConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L77-L78
    if "provider_infos" in argument:
        return {"calls": [provider_answer(config, call) for call in argument["provider_infos"]]}
    if "complete_urls" in argument:
        return {"calls": [complete_url_answer(config, call) for call in argument["complete_urls"]]}
    raise ValueError("expected a foundry agent provider variant")


def handler_reports(handler: AgentHandler, argument: Input) -> JsonValue:
    if "preparations" in argument:
        answers = [preparation_answer(handler, call) for call in argument["preparations"]]
    elif "checks" in argument:
        answers = [check_answer(handler, call) for call in argument["checks"]]
    elif "paths" in argument:
        answers = [path_answer(handler, call) for call in argument["paths"]]
    else:
        raise ValueError("expected a foundry agent handler variant")
    return {"calls": answers}


def run(argument: Input) -> JsonValue:
    if "preparations" in argument or "checks" in argument or "paths" in argument:
        handler = AgentHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L122-L123
        return handler_reports(handler, argument)
    if "provider_infos" in argument or "complete_urls" in argument:
        return provider_reports(argument)
    return config_reports(argument)
