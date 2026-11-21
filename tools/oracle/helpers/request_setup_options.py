import logging
import os
from typing import Final, Literal, TypedDict
from urllib.parse import urlencode

import litellm
from litellm.integrations.otel.model.config import is_otel_v2_enabled
from litellm.proxy import proxy_server
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.litellm_pre_call_utils import (
    _dynamically_disabled_backends,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1047
    _get_dynamic_logging_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L973
    apply_missing_session_id_policy,
    resolve_tenant_otel_destinations,
    safe_add_api_version_from_query_params,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L878
    should_auto_drop_params_for_agentic_cli,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L861
)
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope
from typing_extensions import override

PROXY_LOGGER: Final = "LiteLLM Proxy"


class Principal(TypedDict):
    metadata: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue] | None
    team_id: str | None


class PrepareCall(TypedDict):
    data: dict[str, JsonValue]
    slot: Literal["metadata", "litellm_metadata"]
    route: str
    query: list[tuple[str, str]]
    user_agent: str
    missing_session_id: str | None
    principal: Principal
    default_team_settings: list[dict[str, JsonValue]]


class DestinationsCall(TypedDict):
    principal: Principal
    headers: list[tuple[str, str]]
    callback_disabling: bool
    otel_v2: bool


class Input(TypedDict):
    prepare: list[PrepareCall]
    destinations: list[DestinationsCall]


class CallbackSettings(TypedDict):
    success_callback: list[str] | None
    failure_callback: list[str] | None
    callbacks: list[str] | None
    callback_vars: dict[str, str] | None


class Destination(TypedDict):
    endpoint: str
    headers: dict[str, str]
    resource_attributes: dict[str, str]
    callback_name: str | None
    protocol: str | None
    span_scope: Literal["full", "llm_only"]


class PrepareOutcome(TypedDict):
    data: dict[str, JsonValue]
    callback_settings: CallbackSettings | None


class PrepareAnswer(TypedDict):
    outcome: PrepareOutcome
    warnings: list[str]


class ErrorFacts(TypedDict):
    status: int
    type: str
    param: str | None
    code: str


class ErrorAnswer(TypedDict):
    error: ErrorFacts


class DestinationsAnswer(TypedDict):
    disabled: list[str]
    destinations: list[Destination]
    warnings: list[str]


class Answer(TypedDict):
    prepare: list[JsonValue]
    destinations: list[JsonValue]


JSON: Final = TypeAdapter(JsonValue)
BODY: Final = TypeAdapter(dict[str, JsonValue])
AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)
CALLBACK_SETTINGS: Final = TypeAdapter(CallbackSettings)
DESTINATION: Final = TypeAdapter(Destination)


# the warnings LiteLLM's proxy logger emits, in order; their wording is LiteLLM's own
class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.WARNING)
        self.levels: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.levels.append(record.levelname)


def request_for(route: str, query: list[tuple[str, str]]) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    scope: Scope = {
        "type": "http",
        "scheme": "http",
        "method": "POST",
        "path": route,
        # the case JSON brings each pair as a list, and urlencode takes only tuples
        "query_string": urlencode([(name, value) for name, value in query]).encode(),
        "headers": headers,
    }
    return Request(scope=scope)


def proxy_config_for(teams: list[dict[str, JsonValue]]) -> proxy_server.ProxyConfig:
    proxy_config = proxy_server.ProxyConfig()
    proxy_config.update_config_state({"litellm_settings": {"default_team_settings": teams}})
    return proxy_config


def prepare_outcome(call: PrepareCall) -> PrepareOutcome:
    data = dict[str, object](call["data"])
    request = request_for(call["route"], call["query"])
    safe_add_api_version_from_query_params(data, request)
    apply_missing_session_id_policy(
        data, call["slot"], {"missing_session_id": call["missing_session_id"]}, request
    )
    proxy_config = proxy_config_for(call["default_team_settings"])
    if should_auto_drop_params_for_agentic_cli(call["user_agent"], data, proxy_config):
        data["drop_params"] = True
    auth = AUTH.validate_python(call["principal"])
    settings = _get_dynamic_logging_metadata(auth, proxy_config)
    callback_settings = (
        None
        if settings is None
        else CALLBACK_SETTINGS.validate_json(settings.model_dump_json())
    )
    return PrepareOutcome(data=BODY.validate_python(data), callback_settings=callback_settings)


def prepare(call: PrepareCall, warnings: Warnings) -> JsonValue:
    warnings.levels.clear()
    try:
        outcome = prepare_outcome(call)
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        facts = ErrorFacts(
            status=int(error.code),
            type=error.type,
            param=error.param,
            code=error.code,
        )
        return JSON.validate_python(ErrorAnswer(error=facts))
    levels: list[str] = [*warnings.levels]
    return JSON.validate_python(PrepareAnswer(outcome=outcome, warnings=levels))


def destinations(call: DestinationsCall, warnings: Warnings) -> JsonValue:
    warnings.levels.clear()
    litellm.allow_dynamic_callback_disabling = call["callback_disabling"]
    # betterlite has no license gate, so LiteLLM runs as the premium proxy the deviation names
    proxy_server.premium_user = True
    os.environ["LITELLM_OTEL_V2"] = "true" if call["otel_v2"] else "false"
    is_otel_v2_enabled.cache_clear()
    auth = AUTH.validate_python(call["principal"])
    headers = dict(call["headers"])
    disabled = sorted(_dynamically_disabled_backends(auth, headers))
    found = [
        DESTINATION.validate_json(destination.model_dump_json())
        for destination in resolve_tenant_otel_destinations(auth, headers)
    ]
    levels: list[str] = [*warnings.levels]
    return JSON.validate_python(
        DestinationsAnswer(disabled=disabled, destinations=found, warnings=levels)
    )


def run(input_value: Input) -> JsonValue:
    logger = logging.getLogger(PROXY_LOGGER)
    logger.setLevel(logging.WARNING)
    warnings = Warnings()
    logger.addHandler(warnings)
    try:
        outcome = Answer(
            prepare=[prepare(call, warnings) for call in input_value["prepare"]],
            destinations=[destinations(call, warnings) for call in input_value["destinations"]],
        )
    finally:
        logger.removeHandler(warnings)
    return JSON.validate_python(outcome)
