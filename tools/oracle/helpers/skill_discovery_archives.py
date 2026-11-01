import base64
from typing import TypeAlias

from litellm.proxy.discovery_endpoints.agent_skills_archive import build_skill_archive

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"stored_content"} or not isinstance(input_value["stored_content"], str):
        raise ValueError("stored_content must be a base64 string")
    stored_content = base64.b64decode(input_value["stored_content"])
    archive = build_skill_archive(stored_content)
    if archive is None:
        return None
    return {
        "content": base64.b64encode(archive.content).decode("ascii"),
        "digest": archive.digest,
        "declared_name": archive.declared_name,
        "declared_description": archive.declared_description,
    }
