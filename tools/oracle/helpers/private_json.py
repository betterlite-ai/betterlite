import json
import stat
import tempfile
from pathlib import Path
from typing import TypeAlias, cast

import litellm
from litellm.constants import DEFAULT_MAX_RECURSE_DEPTH
from litellm.litellm_core_utils.private_json import (
    commit_staged_json,
    discard_staged_json,
    ensure_private_dir,
    overwrite_private_json,
    stage_private_json,
    write_private_json,
)
from litellm.litellm_core_utils.safe_json_dumps import safe_json_structure
from litellm.litellm_core_utils.safe_json_loads import safe_json_loads

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
_MISSING: object = object()


def run(input_value: dict[str, Json]) -> Json:
    function = _text(input_value.get("function"), "function")
    if function == "structure":
        return _structure(input_value)
    if function == "load":
        return _load(input_value)
    return _file(function, input_value)


def _structure(input_value: Fields) -> Json:
    data = cast(object, input_value.get("data"))
    max_depth = input_value.get("max_depth")
    depth = DEFAULT_MAX_RECURSE_DEPTH if max_depth is None else _integer(max_depth, "max_depth")
    return cast(Json, safe_json_structure(data, depth))


def _load(input_value: Fields) -> Json:
    text = _text(input_value.get("text"), "text")
    parsed = cast(Json, safe_json_loads(text, _MISSING))
    if parsed is _MISSING:
        raise ValueError(f"{litellm.__name__} safe_json_loads returned its default")
    return parsed


def _file(function: str, input_value: Fields) -> Json:
    name = _text(input_value.get("name"), "name")
    with tempfile.TemporaryDirectory() as root:
        directory = Path(root)
        if function == "stage_commit":
            return _stage_commit(directory, name, _object(input_value.get("data"), "data"))
        if function == "stage_discard":
            return _stage_discard(directory, name, _object(input_value.get("data"), "data"))
        if function == "overwrite":
            return _overwrite(directory, name, _object(input_value.get("data"), "data"))
        if function == "owner_permissions":
            return _owner_permissions(directory, name, _object(input_value.get("data"), "data"))
        if function == "symlinked_parent":
            return _symlinked_parent(directory, name, _object(input_value.get("data"), "data"))
        if function == "ensure_private_dir":
            return _ensure_private_dir(directory, name)
    raise ValueError(f"unknown {litellm.__name__} private_json function {function}")


def _stage_commit(root: Path, name: str, data: Fields) -> Json:
    target = root / name
    staged = stage_private_json(str(target), data)
    commit_staged_json(staged, str(target))
    return _file_view(target)


def _stage_discard(root: Path, name: str, data: Fields) -> Json:
    target = root / name
    staged = stage_private_json(str(target), data)
    discard_staged_json(staged)
    return _file_view(target)


def _overwrite(root: Path, name: str, data: Fields) -> Json:
    target = root / name
    target.write_text('{"old": true}', encoding="utf-8")
    target.chmod(0o640)
    overwrite_private_json(str(target), data)
    return _file_view(target)


def _owner_permissions(root: Path, name: str, data: Fields) -> Json:
    directory = root / "conf"
    ensure_private_dir(directory)
    target = directory / name
    write_private_json(str(target), data)
    return {
        "directory_mode": stat.S_IMODE(directory.stat().st_mode),
        "file": _file_view(target),
    }


def _symlinked_parent(root: Path, name: str, data: Fields) -> Json:
    real = root / "real"
    real.mkdir()
    link = root / "link"
    link.symlink_to(real)
    target = link / name
    write_private_json(str(target), data)
    return {"real": _file_view(real / name), "parent_is_symlink": link.is_symlink()}


def _ensure_private_dir(root: Path, name: str) -> Json:
    created = root / name
    ensure_private_dir(created)
    tightened = root / f"{name}-tightened"
    tightened.mkdir()
    tightened.chmod(0o777)
    ensure_private_dir(tightened)
    return {
        "created_mode": stat.S_IMODE(created.stat().st_mode),
        "tightened_mode": stat.S_IMODE(tightened.stat().st_mode),
    }


def _file_view(target: Path) -> Json:
    leftovers = _leftovers(target)
    if not target.exists():
        return {"exists": False, "file_mode": None, "content": None, "leftovers": leftovers}
    return {
        "exists": True,
        "file_mode": stat.S_IMODE(target.stat().st_mode),
        "content": cast(Json, json.loads(target.read_text(encoding="utf-8"))),
        "leftovers": leftovers,
    }


def _leftovers(target: Path) -> int:
    return len([entry for entry in target.parent.iterdir() if entry.name != target.name])


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _object(value: Json, name: str) -> Fields:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    return value
