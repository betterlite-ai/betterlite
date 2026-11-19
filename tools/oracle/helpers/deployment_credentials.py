import logging
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm.router import Router
from litellm.types.utils import CredentialItem
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

ROUTER_LOGGER: Final = "LiteLLM Router"
CREDENTIALS: Final[TypeAdapter[list[CredentialItem]]] = TypeAdapter(list[CredentialItem])
OPTIONAL: Final[TypeAdapter[dict[str, JsonValue] | None]] = TypeAdapter(
    dict[str, JsonValue] | None
)


class Arguments(TypedDict, total=False):
    model_id: str
    team_id: str | None


class CredentialRow(TypedDict):
    credential_name: str
    credential_values: dict[str, JsonValue]


class PatternRow(TypedDict):
    pattern: str
    deployment: dict[str, JsonValue]


class Input(TypedDict):
    function: Literal[
        "get_deployment_credentials", "get_deployment_credentials_with_provider"
    ]
    model_list: list[dict[str, JsonValue]]
    arguments: Arguments
    credentials: NotRequired[list[CredentialRow]]
    # a pattern deployment the case adds raw, which Router construction would have validated
    pattern_deployments: NotRequired[list[PatternRow]]


class WarningLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


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


def install_patterns(router: Router, rows: list[PatternRow]) -> None:
    for row in rows:
        router.pattern_router.add_pattern(row["pattern"], row["deployment"])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pattern_match_deployments.py#L63


def case_answer(router: Router, call: Input, log: WarningLog) -> JsonValue:
    arguments = call["arguments"]
    model_id = arguments.get("model_id")
    if model_id is None:
        raise ValueError("arguments.model_id is required")
    team_id = arguments.get("team_id")
    if call["function"] == "get_deployment_credentials":
        selected = router.get_deployment(model_id)
        raw = router.get_deployment_credentials(model_id)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L10286-L10299
    else:
        selected = router.get_credential_deployment(model_id, team_id)
        raw = router.get_deployment_credentials_with_provider(model_id, team_id)
    credentials = OPTIONAL.validate_python(raw)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L10661
    warnings: list[JsonValue] = []
    warnings.extend(log.messages)
    carried: JsonValue = {
        "id": None if selected is None else selected.model_info.id,
        "credentials": credentials,
        "warnings": warnings,
    }
    return carried


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    previous_credentials = litellm.credential_list
    log = WarningLog()
    logger = logging.getLogger(ROUTER_LOGGER)
    try:
        credentials = input_value.get("credentials")
        if credentials is not None:
            install_credentials(credentials)
        router = Router(model_list=input_value["model_list"])
        patterns = input_value.get("pattern_deployments")
        if patterns is not None:
            install_patterns(router, patterns)
        previous_level = logger.level
        logger.addHandler(log)
        logger.setLevel(logging.WARNING)
        try:
            return case_answer(router, input_value, log)
        finally:
            logger.removeHandler(log)
            logger.setLevel(previous_level)
    finally:
        litellm.credential_list = previous_credentials
