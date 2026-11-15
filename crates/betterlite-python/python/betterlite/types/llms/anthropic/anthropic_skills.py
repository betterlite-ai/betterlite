from typing import ClassVar

from pydantic import ConfigDict, JsonValue
from typing_extensions import TypedDict

from betterlite.types.utils.base import ResponseObject


class CreateSkillRequest(TypedDict, total=False):
    display_title: str | None
    files: list[JsonValue] | None


class ListSkillsParams(TypedDict, total=False):
    limit: int | None
    page: str | None
    source: str | None


class Skill(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    id: str
    created_at: str
    display_title: str | None = None
    latest_version: str | None = None
    source: str
    type: str = "skill"
    updated_at: str
    description: str | None = None
    search_score: float | None = None


class ListSkillsResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    data: list[Skill]
    next_page: str | None = None
    has_more: bool = False


class DeleteSkillResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    id: str
    type: str = "skill_deleted"


class CreateSkillVersionRequest(TypedDict, total=False):
    display_title: str | None
    description: str | None
    instructions: str | None
    metadata: dict[str, JsonValue] | None


class SkillVersion(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    id: str
    skill_id: str
    created_at: str
    display_title: str | None = None
    description: str | None = None
    instructions: str | None = None
    metadata: dict[str, JsonValue] | None = None
    type: str = "skill.version"


class ListSkillVersionsResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    object: str = "list"
    data: list[SkillVersion]
    first_id: str | None = None
    last_id: str | None = None
    has_more: bool = False


class DeleteSkillVersionResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    id: str
    object: str = "skill.version.deleted"
    deleted: bool
