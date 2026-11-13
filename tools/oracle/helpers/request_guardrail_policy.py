import asyncio
import copy
import logging
import sys
from types import ModuleType
from typing import Final, NotRequired, TypedDict

from litellm.proxy import litellm_pre_call_utils
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.policy_engine.attachment_registry import get_attachment_registry
from litellm.proxy.policy_engine.policy_registry import get_policy_registry
from litellm.types.proxy.policy_engine import Policy
from pydantic import JsonValue
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
# the logger name is public even though litellm._logging is not
PROXY_LOGGER: Final = "LiteLLM Proxy"  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/_logging.py#L869
POLICY_NOT_FOUND_PREFIX: Final = "Policy engine: policy '"
POLICY_NOT_FOUND_SUFFIX: Final = "' not found in registry"


class Call(TypedDict):
    data: dict[str, JsonValue]
    key_metadata: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue]
    project_metadata: dict[str, JsonValue]
    team_alias: str | None
    key_alias: str | None
    metadata_variable_name: str
    premium_user: bool
    policies: dict[str, JsonValue] | None
    attachments: list[dict[str, JsonValue]] | None
    max_items: NotRequired[int]


class Input(TypedDict):
    calls: list[Call]


class ProxyServer(ModuleType):
    premium_user: bool = False


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.policies: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(POLICY_NOT_FOUND_PREFIX) and message.endswith(
            POLICY_NOT_FOUND_SUFFIX
        ):
            name = message.removeprefix(POLICY_NOT_FOUND_PREFIX)
            self.policies.append(name.removesuffix(POLICY_NOT_FOUND_SUFFIX))


def install_registries(call: Call) -> None:
    policy_registry = get_policy_registry()
    policy_registry.clear()
    policies = call["policies"]
    if policies is not None:
        policy_registry.load_policies({})
        for name, definition in policies.items():
            policy_registry.add_policy(
                name, Policy.model_validate(definition), "config"
            )
    attachment_registry = get_attachment_registry()
    attachment_registry.clear()
    attachments = call["attachments"]
    if attachments is not None:
        attachment_registry.load_attachments(attachments)


def build_auth(call: Call) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        metadata=call["key_metadata"],
        team_metadata=call["team_metadata"],
        project_metadata=call["project_metadata"],
        team_alias=call["team_alias"],
        key_alias=call["key_alias"],
    )


def call_value(call: Call) -> dict[str, JsonValue]:
    install_registries(call)
    auth = build_auth(call)
    data = copy.deepcopy(call["data"])
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.premium_user = call["premium_user"]
    saved_module = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_server
    warnings = Warnings()
    logger = logging.getLogger(PROXY_LOGGER)
    saved_level = logger.level
    logger.addHandler(warnings)
    logger.setLevel(logging.WARNING)
    try:
        asyncio.run(
            litellm_pre_call_utils.move_guardrails_to_metadata(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L3159
                data, call["metadata_variable_name"], auth
            )
        )
    finally:
        logger.removeHandler(warnings)
        logger.setLevel(saved_level)
        if saved_module is None:
            sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = saved_module
    excluded: list[JsonValue] = list[JsonValue](sorted(warnings.policies))
    payload: dict[str, JsonValue] = {"data": data, "excluded": excluded}
    return payload


def run(input_value: Input) -> list[JsonValue]:
    results: list[JsonValue] = [call_value(call) for call in input_value["calls"]]
    return results
