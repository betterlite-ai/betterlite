import logging
from collections.abc import Iterator
from contextlib import contextmanager
from typing import Final, NotRequired, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.policy_engine.attachment_registry import get_attachment_registry
from litellm.proxy.policy_engine.policy_registry import get_policy_registry
from litellm.proxy.policy_engine.response_retrieval import (
    attach_post_call_pipelines_to_retrieval,
)
from litellm.router import Router
from litellm.types.proxy.policy_engine import Policy, PolicyAttachment
from litellm.types.proxy.policy_engine.pipeline_types import GuardrailPipeline
from litellm.types.router import RouterModelGroupAliasItem
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# LiteLLM's verbose_proxy_logger writes to the logger named "LiteLLM Proxy"
POLICY_LOGGER: Final = "LiteLLM Proxy"
POLICY_PREFIX: Final = "Policy engine: "


class Auth(TypedDict, total=False):
    team_alias: str | None
    key_alias: str | None


class RouterInput(TypedDict):
    deployments: dict[str, str]
    aliases: list[tuple[str, str]]


class RequestData(TypedDict, total=False):
    response_id: str | None
    tags: list[str]
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None


class Metadata(TypedDict, total=False):
    _guardrail_pipelines: list[tuple[str, GuardrailPipeline]] | None
    _pipeline_managed_guardrails: list[str] | None
    applied_policies: list[str] | None
    applied_guardrails: list[str] | None
    policy_sources: dict[str, str] | None


class Input(TypedDict):
    data: RequestData
    auth: Auth
    router: NotRequired[RouterInput | None]
    policies: NotRequired[dict[str, Policy] | None]
    attachments: list[PolicyAttachment]


class LogEntry(TypedDict):
    level: str
    message: str


class Output(TypedDict):
    data: dict[str, JsonValue]
    logs: list[LogEntry]


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)
METADATA: Final[TypeAdapter[Metadata]] = TypeAdapter(Metadata)
DEPLOYMENTS: Final[TypeAdapter[list[dict[str, JsonValue]]]] = TypeAdapter(
    list[dict[str, JsonValue]]
)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


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
            self.entries.append({"level": "warning", "message": message})
        elif record.levelno == logging.DEBUG:
            self.entries.append({"level": "debug", "message": message})


@contextmanager
def capturing(logger: logging.Logger) -> Iterator[Warnings]:
    warnings = Warnings()
    level = logger.level
    handlers = logger.handlers[:]
    logger.handlers = [warnings]
    logger.setLevel(logging.DEBUG)
    try:
        yield warnings
    finally:
        logger.handlers = handlers
        logger.setLevel(level)


def install(call: Input) -> None:
    registry = get_policy_registry()
    registry.clear()
    policies = call.get("policies")
    if policies is not None:
        for name, policy in policies.items():
            registry.add_policy(name, policy, source="config")
    attachments = get_attachment_registry()
    attachments.clear()
    for attachment in call["attachments"]:
        attachments.add_attachment(attachment)


def build_router(call: RouterInput | None) -> Router | None:
    if call is None:
        return None
    deployments = DEPLOYMENTS.validate_python(
        [
            {
                "model_name": model_name,
                "litellm_params": {"model": "openai/gpt-4o"},
                "model_info": {"id": model_id},
            }
            for model_id, model_name in call["deployments"].items()
        ]
    )
    aliases: dict[str, str | RouterModelGroupAliasItem] = {}
    aliases.update(call["aliases"])
    return Router(model_list=deployments, model_group_alias=aliases)


def rendered_bucket(base: dict[str, JsonValue], view: Metadata) -> dict[str, JsonValue]:
    rendered = dict(base)
    pipelines = view.get("_guardrail_pipelines")
    if pipelines is not None:
        rendered["_guardrail_pipelines"] = JSON.validate_python(
            [
                [policy_name, JSON.validate_json(pipeline.model_dump_json())]
                for policy_name, pipeline in pipelines
            ]
        )
    managed = view.get("_pipeline_managed_guardrails")
    if managed is not None:
        rendered["_pipeline_managed_guardrails"] = JSON.validate_python(sorted(managed))
    policies = view.get("applied_policies")
    if policies is not None:
        rendered["applied_policies"] = JSON.validate_python(policies)
    guardrails = view.get("applied_guardrails")
    if guardrails is not None:
        rendered["applied_guardrails"] = JSON.validate_python(guardrails)
    sources = view.get("policy_sources")
    if sources is not None:
        rendered["policy_sources"] = JSON.validate_python(sources)
    return rendered


def run(call: Input) -> JsonValue:
    validated = INPUT.validate_python(call)
    install(validated)
    router = build_router(validated.get("router"))
    auth = UserAPIKeyAuth(
        team_alias=validated["auth"].get("team_alias"),
        key_alias=validated["auth"].get("key_alias"),
    )
    request = validated["data"]
    data = dict(request)
    with capturing(logging.getLogger(POLICY_LOGGER)) as warnings:
        attach_post_call_pipelines_to_retrieval(data, auth, router)
    rendered: dict[str, JsonValue] = {
        key: JSON.validate_python(value)
        for key, value in request.items()
        if key not in ("metadata", "litellm_metadata")
    }
    metadata = request.get("metadata")
    if metadata is not None:
        rendered["metadata"] = rendered_bucket(metadata, METADATA.validate_python(metadata))
    else:
        created = data.get("metadata")
        if created is not None:
            rendered["metadata"] = rendered_bucket({}, METADATA.validate_python(created))
        elif "metadata" in request:
            rendered["metadata"] = None
    litellm_metadata = request.get("litellm_metadata")
    if litellm_metadata is not None:
        rendered["litellm_metadata"] = rendered_bucket(
            litellm_metadata, METADATA.validate_python(litellm_metadata)
        )
    else:
        created_litellm = data.get("litellm_metadata")
        if created_litellm is not None:
            rendered["litellm_metadata"] = rendered_bucket(
                {}, METADATA.validate_python(created_litellm)
            )
        elif "litellm_metadata" in request:
            rendered["litellm_metadata"] = None
    output: Output = {"data": rendered, "logs": warnings.entries}
    return JSON.validate_python(output)
