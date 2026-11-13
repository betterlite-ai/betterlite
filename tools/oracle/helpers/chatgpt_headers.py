import os
import platform
from typing import TypedDict
from unittest.mock import patch

from litellm.llms.chatgpt import common_utils

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

ENVIRONMENT_NAMES = (
    "CHATGPT_ORIGINATOR",
    "CHATGPT_USER_AGENT",
    "CHATGPT_USER_AGENT_SUFFIX",
    "CHATGPT_DEFAULT_INSTRUCTIONS",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "WEZTERM_VERSION",
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    "ITERM_PROFILE_NAME",
    "TERM_SESSION_ID",
    "KITTY_WINDOW_ID",
    "TERM",
    "ALACRITTY_SOCKET",
    "KONSOLE_VERSION",
    "GNOME_TERMINAL_SCREEN",
    "VTE_VERSION",
    "WT_SESSION",
)


class Input(TypedDict):
    platform: dict[str, str]
    environment: dict[str, str]
    access_token: str
    account_id: str | None
    session_id: str | None
    litellm_params: Json


class FailingDumpSnapshot:
    def __init__(self, dict_fields: dict[str, Json]) -> None:
        self.dict_fields: dict[str, Json] = dict_fields

    def model_dump(self) -> dict[str, Json]:
        raise RuntimeError("model_dump is unavailable on this snapshot")

    def dict(self) -> dict[str, Json]:
        return self.dict_fields


def check_environment(environment: dict[str, str]) -> None:
    observed = {name: os.environ[name] for name in ENVIRONMENT_NAMES if name in os.environ}
    if observed != environment:
        raise ValueError("the call environment must match the case env")


def model_snapshot(spec: dict[str, Json]) -> FailingDumpSnapshot:
    model_dump_tag = spec.get("model_dump")
    if model_dump_tag != "failed":
        raise ValueError("only a failing model_dump is exercised by this helper")
    dict_tag = spec.get("dict")
    if not isinstance(dict_tag, dict):
        raise ValueError("dict outcome must be an object")
    fields = dict_tag.get("succeeded")
    if not isinstance(fields, dict):
        raise ValueError("only a succeeding dict outcome is exercised by this helper")
    return FailingDumpSnapshot(fields)


def litellm_params_value(spec: Json) -> object:
    if spec is None or spec == "absent":
        return None
    if not isinstance(spec, dict):
        raise ValueError("litellm_params must be an object or null")
    if "mapping" in spec:
        mapping = spec["mapping"]
        if not isinstance(mapping, dict):
            raise ValueError("mapping outcome must be an object")
        return mapping
    if "model_snapshot" in spec:
        snapshot_spec = spec["model_snapshot"]
        if not isinstance(snapshot_spec, dict):
            raise ValueError("model_snapshot outcome must be an object")
        return model_snapshot(snapshot_spec)
    raise ValueError("litellm_params must carry mapping or model_snapshot")


def run(input_value: Input) -> Json:
    check_environment(input_value["environment"])
    pinned = input_value["platform"]
    with (
        patch.object(platform, "system", return_value=pinned["system"]),
        patch.object(platform, "release", return_value=pinned["release"]),
        patch.object(platform, "machine", return_value=pinned["machine"]),
    ):
        originator = common_utils.get_chatgpt_originator()
        user_agent = common_utils.get_chatgpt_user_agent(originator)
        headers: dict[str, Json] = common_utils.get_chatgpt_default_headers(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            input_value["access_token"],
            input_value["account_id"],
            input_value["session_id"],
        )
        instructions = common_utils.get_chatgpt_default_instructions()
        litellm_params = litellm_params_value(input_value["litellm_params"])
        session_id = common_utils.get_chatgpt_session_id(litellm_params)
    return {
        "originator": originator,
        "user_agent": user_agent,
        "headers": headers,
        "instructions": instructions,
        "session_id": session_id,
    }
