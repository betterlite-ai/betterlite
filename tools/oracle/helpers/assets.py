import contextlib
import io
import logging
import tempfile
from pathlib import Path
from typing import Literal, NotRequired, TypedDict

from litellm._logging import verbose_proxy_logger
from litellm.proxy.common_utils import path_utils, static_asset_utils

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class DirEntry(TypedDict):
    kind: Literal["dir"]
    path: str


class FileEntry(TypedDict):
    kind: Literal["file"]
    path: str
    content_hex: NotRequired[str]


class SymlinkEntry(TypedDict):
    kind: Literal["symlink"]
    path: str
    target: str


Entry = DirEntry | FileEntry | SymlinkEntry


class SafeJoinInput(TypedDict):
    op: Literal["safe_join"]
    fixture: NotRequired[list[Entry]]
    parts: list[str]


class TrySafeJoinInput(TypedDict):
    op: Literal["try_safe_join"]
    fixture: NotRequired[list[Entry]]
    parts: list[str]


class IsWithinInput(TypedDict):
    op: Literal["is_within"]
    fixture: NotRequired[list[Entry]]
    path: str


class JoinWithinInput(TypedDict):
    op: Literal["join_within"]
    base_dir: str
    parts: list[str]


class SafeFilenameInput(TypedDict):
    op: Literal["safe_filename"]
    filename: str


class DetectLocalImageMediaTypeInput(TypedDict):
    op: Literal["detect_local_image_media_type"]
    header_hex: str


class ResolveLocalImageInput(TypedDict):
    op: Literal["resolve_local_image"]
    fixture: NotRequired[list[Entry]]
    candidate: str


Input = (
    SafeJoinInput
    | TrySafeJoinInput
    | IsWithinInput
    | JoinWithinInput
    | SafeFilenameInput
    | DetectLocalImageMediaTypeInput
    | ResolveLocalImageInput
)


def run(input_value: Input) -> Json:
    if input_value["op"] == "safe_join" or input_value["op"] == "try_safe_join":
        return _run_safe_join(
            input_value.get("fixture", []),
            input_value["parts"],
            strict=input_value["op"] == "safe_join",
        )
    if input_value["op"] == "is_within":
        return _run_is_within(input_value.get("fixture", []), input_value["path"])
    if input_value["op"] == "join_within":
        return path_utils.join_within(input_value["base_dir"], *input_value["parts"])
    if input_value["op"] == "safe_filename":
        return path_utils.safe_filename(input_value["filename"])
    if input_value["op"] == "detect_local_image_media_type":
        header = bytes.fromhex(input_value["header_hex"])
        return static_asset_utils.detect_local_image_media_type(header)
    return _run_resolve_local_image(input_value.get("fixture", []), input_value["candidate"])


def _materialize(root: Path, entries: list[Entry]) -> None:
    for entry in entries:
        full = root / entry["path"]
        if entry["kind"] == "dir":
            full.mkdir(parents=True, exist_ok=True)
        elif entry["kind"] == "file":
            full.parent.mkdir(parents=True, exist_ok=True)
            full.write_bytes(bytes.fromhex(entry.get("content_hex", "")))
        else:
            full.parent.mkdir(parents=True, exist_ok=True)
            full.symlink_to(entry["target"])


def _relative_to(root: Path, target: Path) -> str | None:
    try:
        relative = target.relative_to(root)
    except ValueError:
        return None
    return str(relative)


def _run_safe_join(entries: list[Entry], parts: list[str], *, strict: bool) -> Json:
    with tempfile.TemporaryDirectory() as root:
        base = Path(root) / "base"
        base.mkdir(parents=True, exist_ok=True)
        _materialize(base, entries)
        canonical_base = base.resolve()
        if strict:
            resolved = path_utils.safe_join(str(base), *parts)
        else:
            resolved = path_utils.try_safe_join(str(base), *parts)
            if resolved is None:
                return None
        relative = _relative_to(canonical_base, Path(resolved))
        if relative is None:
            raise ValueError("resolved path left the fixture root")
        return relative


def _run_is_within(entries: list[Entry], path: str) -> Json:
    with tempfile.TemporaryDirectory() as root:
        base = Path(root) / "base"
        base.mkdir(parents=True, exist_ok=True)
        _materialize(base, entries)
        return path_utils.is_within(str(base / path), str(base))


def _run_resolve_local_image(entries: list[Entry], candidate: str) -> Json:
    with tempfile.TemporaryDirectory() as root:
        root_path = Path(root)
        _materialize(root_path, entries)
        resolved_candidate = candidate.replace("<root>", root)
        levels: list[str] = []
        handler = _LevelHandler(levels)
        previous_level = verbose_proxy_logger.level
        verbose_proxy_logger.setLevel(logging.DEBUG)
        verbose_proxy_logger.addHandler(handler)
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                outcome = static_asset_utils.resolve_validated_local_image_path(resolved_candidate)
        finally:
            verbose_proxy_logger.removeHandler(handler)
            verbose_proxy_logger.setLevel(previous_level)
        log_level = levels[-1] if levels else None
        if outcome is None:
            return {"media_type": None, "relative_path": None, "log_level": log_level}
        resolved_path, media_type = outcome
        relative_path = _relative_to(root_path.resolve(), Path(resolved_path))
        return {"media_type": media_type, "relative_path": relative_path, "log_level": log_level}


class _LevelHandler(logging.Handler):
    def __init__(self, levels: list[str]) -> None:
        super().__init__()
        self.levels = levels

    def emit(self, record: logging.LogRecord) -> None:
        self.levels.append(record.levelname.lower())
