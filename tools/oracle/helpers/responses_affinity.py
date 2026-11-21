import asyncio
import logging
from types import TracebackType
from typing import Final, NotRequired, Self, TypedDict, cast

import litellm
from litellm.router import Router
from litellm.router_utils.pre_call_checks.encrypted_content_affinity_check import (
    EncryptedContentAffinityCheck,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import CredentialItem
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

ROUTER_LOGGER: Final = "LiteLLM Router"
CREDENTIALS: Final[TypeAdapter[list[CredentialItem]]] = TypeAdapter(list[CredentialItem])
METADATA_KEY: Final = "litellm_metadata"
PIN_FLAG: Final = "_encrypted_content_affinity_pinned"


class CredentialRow(TypedDict):
    credential_name: str
    credential_values: dict[str, JsonValue]


class DeployView(TypedDict):
    litellm_params: NotRequired[dict[str, JsonValue]]
    model_info: NotRequired[dict[str, JsonValue]]


class Input(TypedDict):
    model: str
    healthy_deployments: list[DeployView]
    request_kwargs: dict[str, JsonValue]
    enable_global_affinity: NotRequired[bool]
    model_group_affinity_config: NotRequired[dict[str, list[str]]]
    credentials: NotRequired[list[CredentialRow]]
    messages: NotRequired[list[dict[str, JsonValue]]]
    model_list: NotRequired[list[dict[str, JsonValue]]]
    router: NotRequired[bool]


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


class CapturedLogs:
    def __init__(self) -> None:
        self.logger = logging.getLogger(ROUTER_LOGGER)
        self.capture = Capture()
        self.handlers: list[logging.Handler] = []
        self.level = logging.NOTSET
        self.propagate = True

    def __enter__(self) -> Self:
        self.handlers = list(self.logger.handlers)
        for handler in self.handlers:
            self.logger.removeHandler(handler)
        self.level = self.logger.level
        self.propagate = self.logger.propagate
        self.logger.setLevel(logging.DEBUG)
        self.logger.propagate = False
        self.logger.addHandler(self.capture)
        return self

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.logger.removeHandler(self.capture)
        self.logger.setLevel(self.level)
        self.logger.propagate = self.propagate
        for handler in self.handlers:
            self.logger.addHandler(handler)


def install_credentials(rows: list[CredentialRow]) -> None:
    records: list[dict[str, JsonValue]] = [
        {
            "credential_name": row["credential_name"],
            "credential_info": {},
            "credential_values": row["credential_values"],
        }
        for row in rows
    ]
    litellm.credential_list = CREDENTIALS.validate_python(records)


def row_id(row: DeployView) -> str | None:
    info = row.get("model_info")
    if info is None:
        return None
    identifier = info.get("id")
    if identifier is None:
        return None
    return str(identifier)


def case_answer(router: Router, call: Input) -> JsonValue:
    config = call.get("model_group_affinity_config")
    global_flag = call.get("enable_global_affinity")
    router_flag = call.get("router")
    check = EncryptedContentAffinityCheck(
        router=router if router_flag is not False else None,
        enable_global_affinity=True if global_flag is None else global_flag,
        model_group_affinity_config=config,
    )
    kwargs = dict(call["request_kwargs"])
    messages_arg = call.get("messages")
    cast_messages = cast(list[AllMessageValues] | None, messages_arg)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L266-L273
    with CapturedLogs() as logs:
        result = asyncio.run(  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pre_call_checks/encrypted_content_affinity_check.py#L304-L311
            check.async_filter_deployments(  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pre_call_checks/encrypted_content_affinity_check.py#L304-L311
                model=call["model"],
                healthy_deployments=call["healthy_deployments"],
                messages=cast_messages,
                request_kwargs=kwargs,
            )
        )
        rows = cast(list[DeployView], result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pre_call_checks/encrypted_content_affinity_check.py#L304-L311
    carried: JsonValue = {
        "ids": [row_id(row) for row in rows],
        "input": kwargs.get("input"),
        "messages": messages_arg,
        "request_messages": kwargs.get("messages"),
        "metadata": kwargs.get(METADATA_KEY),
        "pinned": kwargs.get(PIN_FLAG) is True,
        "logs": list(logs.capture.messages),
        "group_affinity_enabled": EncryptedContentAffinityCheck.has_model_group_affinity_enabled(config),
    }
    return carried


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    previous_credentials = litellm.credential_list
    try:
        credentials = input_value.get("credentials")
        if credentials is not None:
            install_credentials(credentials)
        router = Router(model_list=input_value.get("model_list"))
        return case_answer(router, input_value)
    finally:
        litellm.credential_list = previous_credentials
