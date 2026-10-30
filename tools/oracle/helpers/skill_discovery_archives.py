import base64
import hashlib
import io
import re
import zipfile
from typing import TypeAlias, cast

import yaml

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
MAX_ARCHIVE_ENTRIES = 1000
MAX_ARCHIVE_UNPACKED_BYTES = 50 * 1024 * 1024


def _frontmatter(manifest: bytes) -> tuple[str | None, str | None]:
    text = manifest.decode("utf-8", errors="replace")
    match = re.match(r"^---\s*\n(.*?)\n---\s*(?:\n|$)", text, re.DOTALL)
    if match is None:
        return None, None
    try:
        parsed = cast(Json | object, yaml.safe_load(match.group(1)))
    except yaml.YAMLError:
        return None, None
    if not isinstance(parsed, dict):
        return None, None
    mapping = cast(dict[str, object], parsed)
    values: list[str | None] = []
    for key in ("name", "description"):
        value = mapping.get(key)
        text = value.strip() or None if isinstance(value, str) else None
        values.append(text)
    return values[0], values[1]


def run(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"stored_content"} or not isinstance(input_value["stored_content"], str):
        raise ValueError("stored_content must be a base64 string")
    stored_content = base64.b64decode(input_value["stored_content"])
    try:
        with zipfile.ZipFile(io.BytesIO(stored_content)) as uploaded:
            infos = tuple(info for info in uploaded.infolist() if not info.is_dir())
            if not infos or len(infos) > MAX_ARCHIVE_ENTRIES or sum(
                info.file_size for info in infos
            ) > MAX_ARCHIVE_UNPACKED_BYTES:
                return None
            normalized: list[tuple[str, bytes]] = []
            for info in infos:
                raw = info.filename
                if (
                    not raw
                    or "\0" in raw
                    or "\\" in raw
                    or raw.startswith("/")
                    or re.match(r"^[A-Za-z]:", raw)
                ):
                    return None
                parts = tuple(part for part in raw.split("/") if part)
                if not parts or any(part in (".", "..") for part in parts):
                    return None
                normalized.append(("/".join(parts), uploaded.read(info)))
    except (zipfile.BadZipFile, OSError, RuntimeError):
        return None
    roots: set[str] = {path.split("/", 1)[0] for path, _ in normalized}
    prefix = (
        next(iter(roots)) + "/"
        if len(roots) == 1 and all("/" in path for path, _ in normalized)
        else ""
    )
    flattened: list[tuple[str, bytes]] = [(path[len(prefix) :], data) for path, data in normalized]
    names: set[str] = {name for name, _ in flattened}
    if "SKILL.md" not in names or len(names) != len(flattened):
        return None
    flattened.sort()
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as repacked:
        for name, data in flattened:
            entry = zipfile.ZipInfo(filename=name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = 0o644 << 16
            repacked.writestr(entry, data)
    content = output.getvalue()
    name, description = _frontmatter(dict(flattened)["SKILL.md"])
    return {
        "content": base64.b64encode(content).decode("ascii"),
        "digest": f"sha256:{hashlib.sha256(content).hexdigest()}",
        "declared_name": name,
        "declared_description": description,
    }
