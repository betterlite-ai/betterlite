from typing import NotRequired, TypedDict

from litellm.proxy.management_helpers import model_allowlist_rename_sync


class Input(TypedDict):
    kind: str
    object_id: str
    team_alias: NotRequired[str | None]


def run(input_value: Input) -> dict[str, tuple[str, ...]]:
    row = model_allowlist_rename_sync._TouchedRow(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/model_allowlist_rename_sync.py#L22
        kind=input_value["kind"],
        object_id=input_value["object_id"],
        team_alias=input_value.get("team_alias"),
    )
    kind = input_value["kind"]
    if kind == "team":
        keys = model_allowlist_rename_sync._team_cache_keys(row)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/model_allowlist_rename_sync.py#L44
    elif kind == "key":
        keys = model_allowlist_rename_sync._key_cache_keys(row)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/model_allowlist_rename_sync.py#L48
    elif kind == "org":
        keys = model_allowlist_rename_sync._org_cache_keys(row)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/model_allowlist_rename_sync.py#L52
    elif kind == "project":
        keys = model_allowlist_rename_sync._project_cache_keys(row)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/model_allowlist_rename_sync.py#L56
    elif kind == "user":
        keys = model_allowlist_rename_sync._user_cache_keys(row)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/model_allowlist_rename_sync.py#L60
    else:
        raise ValueError(f"unsupported model_allowlist_rename kind {kind}")
    return {"keys": keys}
