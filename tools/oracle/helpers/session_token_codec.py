import os
import secrets
import sys
from collections.abc import Callable
from datetime import datetime
from types import ModuleType
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
import litellm.utils
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy.auth import auth_checks
from pydantic import JsonValue

CLI_HOURS_ENVIRONMENTS = ("CLI_JWT_EXPIRATION_HOURS", "LITELLM_CLI_JWT_EXPIRATION_HOURS")
FIELDS = {"salt_key", "master_key", "max_ui_session_budget", "env", "nonce_hex", "now", "steps"}
RECORD_FIELDS = {
    "token",
    "key_name",
    "key_alias",
    "max_budget",
    "expires",
    "models",
    "user_id",
    "team_id",
    "team_alias",
    "team_models",
    "team_model_aliases",
    "rpm_limit",
    "user_role",
    "is_session_token",
    "max_parallel_requests",
}
class UserRow(TypedDict):
    user_id: str
    user_role: str | None
    models: list[JsonValue]
    teams: list[str]


class UiStep(TypedDict):
    op: Literal["ui_token"]
    user: UserRow


class CliStep(TypedDict):
    op: Literal["cli_token"]
    user: UserRow
    team_id: NotRequired[str | None]
    team_alias: NotRequired[str | None]
    team_models: NotRequired[list[str] | None]
    team_model_aliases: NotRequired[dict[str, str] | None]
    max_budget: NotRequired[float | None]


class DecodeStep(TypedDict):
    op: Literal["decode"]
    token: str


Step: TypeAlias = UiStep | CliStep | DecodeStep


class Input(TypedDict):
    salt_key: str | None
    master_key: str | None
    max_ui_session_budget: NotRequired[float | None]
    env: dict[str, str]
    nonce_hex: str
    now: str
    steps: list[Step]


class FixedRandom:
    def __init__(self, value: bytes) -> None:
        self.value = value

    def __call__(self, size: int | None = None) -> bytes:
        return self.value[:size]


class FixedClock:
    def __init__(self, moment: datetime) -> None:
        self.moment = moment

    def __call__(self) -> datetime:
        return self.moment


class ProxyServer(ModuleType):
    master_key: str | None = None


def check_environment(input_value: Input) -> None:
    declared = input_value["env"]
    for name in CLI_HOURS_ENVIRONMENTS:
        if declared.get(name) != os.environ.get(name):
            raise ValueError(f"the case env and the helper input disagree on {name}")
    unknown = set(input_value) - FIELDS
    if unknown:
        raise ValueError(f"unknown session token fields: {sorted(unknown)}")


def ui_token(step: UiStep) -> str:
    user = LiteLLM_UserTable.model_validate(step["user"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/user.py#L19
    return auth_checks.ExperimentalUIJWTToken.get_experimental_ui_login_jwt_auth_token(user)


def cli_token(step: CliStep) -> str:
    user = LiteLLM_UserTable.model_validate(step["user"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/user.py#L19
    return auth_checks.ExperimentalUIJWTToken.get_cli_jwt_auth_token(
        user,
        team_id=step.get("team_id"),
        team_alias=step.get("team_alias"),
        team_models=step.get("team_models"),
        team_model_aliases=step.get("team_model_aliases"),
        max_budget=step.get("max_budget"),
    )


def decode(step: DecodeStep) -> JsonValue:
    record = auth_checks.ExperimentalUIJWTToken.get_key_object_from_ui_hash_key(step["token"])
    if record is None:
        return None
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356
        dict[str, JsonValue], record.model_dump(mode="json", include=RECORD_FIELDS)
    )


def step_value(step: Step) -> JsonValue:
    if step["op"] == "ui_token":
        return ui_token(step)
    if step["op"] == "cli_token":
        return cli_token(step)
    if step["op"] == "decode":
        return decode(step)
    raise ValueError("unknown session token step")


def answer(step: Step) -> JsonValue:
    try:
        return {"ok": True, "value": step_value(step)}
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3724-L3740
        return {"ok": False}


def run(input_value: Input) -> list[JsonValue]:
    check_environment(input_value)
    random = FixedRandom(bytes.fromhex(input_value["nonce_hex"]))
    random_source: Callable[[int | None], bytes] = random
    saved_urandom = os.urandom
    saved_token_bytes = secrets.token_bytes
    os.urandom = random
    secrets.token_bytes = random_source  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3701
    saved_salt = os.environ.pop("LITELLM_SALT_KEY", None)
    if input_value["salt_key"] is not None:
        os.environ["LITELLM_SALT_KEY"] = input_value["salt_key"]
    proxy_server = ProxyServer("litellm.proxy.proxy_server")
    proxy_server.master_key = input_value["master_key"]
    saved_proxy = sys.modules.get("litellm.proxy.proxy_server")
    sys.modules["litellm.proxy.proxy_server"] = proxy_server
    previous_budget = litellm.max_ui_session_budget
    litellm.max_ui_session_budget = input_value.get("max_ui_session_budget")
    saved_clock = litellm.utils.get_utc_datetime
    auth_checks.get_utc_datetime = FixedClock(datetime.fromisoformat(input_value["now"]))  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L145
    try:
        return [answer(step) for step in input_value["steps"]]
    finally:
        auth_checks.get_utc_datetime = saved_clock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L145
        os.urandom = saved_urandom
        secrets.token_bytes = saved_token_bytes
        litellm.max_ui_session_budget = previous_budget
        if saved_proxy is None:
            sys.modules.pop("litellm.proxy.proxy_server", None)
        else:
            sys.modules["litellm.proxy.proxy_server"] = saved_proxy
        os.environ.pop("LITELLM_SALT_KEY", None)
        if saved_salt is not None:
            os.environ["LITELLM_SALT_KEY"] = saved_salt
