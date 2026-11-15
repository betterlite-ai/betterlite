from datetime import datetime
from typing import TypedDict

from litellm.proxy.spend_tracking.key_metadata_recovery import KeyMetadataDict, _is_spend_log_digest, _spend_log_cache_key, _SpendLogDigestRow  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/key_metadata_recovery.py#L133-L148
from pydantic import JsonValue


class WindowInput(TypedDict):
    digest: str
    start: str
    end: str


class RowInput(TypedDict):
    digest: str
    first_alias: str | None
    last_alias: str | None
    first_team: str | None
    last_team: str | None
    first_owner: str | None
    last_owner: str | None


class Input(TypedDict):
    keys: list[str]
    windows: list[WindowInput]
    rows: list[RowInput]


def dump_metadata(meta: KeyMetadataDict) -> dict[str, JsonValue]:
    dumped: dict[str, JsonValue] = {}
    key_alias = meta.get("key_alias")
    if key_alias is not None:
        dumped["key_alias"] = key_alias
    team_id = meta.get("team_id")
    if team_id is not None:
        dumped["team_id"] = team_id
    user_id = meta.get("user_id")
    if user_id is not None:
        dumped["user_id"] = user_id
    return dumped


def run(input_value: Input) -> JsonValue:
    digests: list[JsonValue] = [_is_spend_log_digest(key) for key in input_value["keys"]]
    cache_keys: list[JsonValue] = [
        _spend_log_cache_key(
            window["digest"],
            (datetime.fromisoformat(window["start"]), datetime.fromisoformat(window["end"])),
        )
        for window in input_value["windows"]
    ]
    metadata: list[JsonValue] = [
        dump_metadata(_SpendLogDigestRow.model_validate(row).metadata()) for row in input_value["rows"]
    ]
    return {"digests": digests, "cache_keys": cache_keys, "metadata": metadata}
