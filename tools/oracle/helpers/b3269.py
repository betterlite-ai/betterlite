import logging
from typing import Final, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.policy_engine.attachment_registry import get_attachment_registry
from litellm.proxy.policy_engine.policy_registry import get_policy_registry
from litellm.proxy.policy_engine.response_retrieval import (
    attach_post_call_pipelines_to_retrieval,
)
from litellm.router import Router
from litellm.types.router import DeploymentTypedDict, RouterModelGroupAliasItem
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# LiteLLM's verbose_proxy_logger writes to the logger named "LiteLLM Proxy"
POLICY_LOGGER: Final = "LiteLLM Proxy"
POLICY_PREFIX: Final = "Policy engine: "


class Auth(TypedDict):
    team_alias: str | None
    key_alias: str | None


class RouterInput(TypedDict):
    deployments: dict[str, str]
    aliases: list[tuple[str, str]]


class Input(TypedDict):
    data: dict[str, JsonValue]
    auth: Auth
    router: RouterInput | None
    policies: dict[str, dict[str, JsonValue]] | None
    attachments: list[dict[str, JsonValue]]


class LogEntry(TypedDict):
    level: str
    message: str


class Output(TypedDict):
    data: dict[str, JsonValue]
    logs: list[LogEntry]


BODY: Final = TypeAdapter(dict[str, object])
POLICY_CONFIG: Final = TypeAdapter(dict[str, dict[str, object]])
DECODED: Final = TypeAdapter(dict[str, JsonValue])
JSON: Final = TypeAdapter(JsonValue)
ANY: Final = TypeAdapter(object)
TEXTS: Final = TypeAdapter(list[str])


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.entries: list[LogEntry] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if not message.startswith(POLICY_PREFIX):
            return
        if record.levelno == logging.WARNING:
            self.entries.append(LogEntry(level="warning", message=message))
        elif record.levelno == logging.DEBUG:
            self.entries.append(LogEntry(level="debug", message=message))


def install(call: Input) -> None:
    registry = get_policy_registry()
    registry.clear()
    policies = call["policies"]
    if policies is not None:
        registry.load_policies(POLICY_CONFIG.validate_python(policies))
    attachments = get_attachment_registry()
    attachments.clear()
    attachments.load_attachments(call["attachments"])


def build_router(call: RouterInput | None) -> Router | None:
    if call is None:
        return None
    deployments: list[DeploymentTypedDict] = [
        DeploymentTypedDict(
            model_name=model_name,
            litellm_params={"model": "openai/gpt-4o"},
            model_info={"id": model_id},
        )
        for model_id, model_name in call["deployments"].items()
    ]
    aliases: dict[str, str | RouterModelGroupAliasItem] = {}
    aliases.update(call["aliases"])
    return Router(model_list=deployments, model_group_alias=aliases)


def sorted_managed(decoded: dict[str, JsonValue]) -> dict[str, JsonValue]:
    normalized: dict[str, JsonValue] = dict(decoded)
    for name in ("metadata", "litellm_metadata"):
        value = normalized.get(name)
        if value is not None:
            bucket = DECODED.validate_python(value)
            managed = bucket.get("_pipeline_managed_guardrails")
            if managed is not None:
                bucket["_pipeline_managed_guardrails"] = sorted(TEXTS.validate_python(managed))
            normalized[name] = bucket
    return normalized


def run(call: Input) -> JsonValue:
    install(call)
    router = build_router(call["router"])
    auth = UserAPIKeyAuth(
        team_alias=call["auth"]["team_alias"],
        key_alias=call["auth"]["key_alias"],
    )
    data = BODY.validate_python(call["data"])
    warnings = Warnings()
    logger = logging.getLogger(POLICY_LOGGER)
    level = logger.level
    handlers = logger.handlers[:]
    logger.handlers = [warnings]
    logger.setLevel(logging.DEBUG)
    try:
        attach_post_call_pipelines_to_retrieval(data, auth, router)
    finally:
        logger.handlers = handlers
        logger.setLevel(level)
    decoded = DECODED.validate_json(ANY.dump_json(data))
    output: Output = {"data": sorted_managed(decoded), "logs": warnings.entries}
    return JSON.validate_python(output)
