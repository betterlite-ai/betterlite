from typing import ClassVar

from pydantic import ConfigDict, Field

from betterlite.types.proxy.policy_engine.pipeline_types import GuardrailPipeline
from betterlite.types.utils.base import ResponseObject


class PolicyCondition(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    model: str | list[str] | None = None


class PolicyScope(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    teams: list[str] | None = None
    keys: list[str] | None = None
    models: list[str] | None = None
    tags: list[str] | None = None

    def get_teams(self) -> list[str]:
        if not self.teams:
            return ["*"]
        return self.teams

    def get_keys(self) -> list[str]:
        if not self.keys:
            return ["*"]
        return self.keys

    def get_models(self) -> list[str]:
        if not self.models:
            return ["*"]
        return self.models

    def get_tags(self) -> list[str]:
        if not self.tags:
            return []
        return self.tags


class PolicyGuardrails(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    add: list[str] | None = None
    remove: list[str] | None = None

    def get_add(self) -> list[str]:
        if not self.add:
            return []
        return self.add

    def get_remove(self) -> list[str]:
        if not self.remove:
            return []
        return self.remove


class Policy(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    inherit: str | None = None
    description: str | None = None
    guardrails: PolicyGuardrails = Field(default_factory=PolicyGuardrails)
    condition: PolicyCondition | None = None
    pipeline: GuardrailPipeline | None = None


class PolicyAttachment(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    policy: str
    scope: str | None = None
    teams: list[str] | None = None
    keys: list[str] | None = None
    models: list[str] | None = None
    tags: list[str] | None = None
    priority: int | None = Field(default=None, ge=-2147483648, le=2147483647)
    default: bool = False

    def is_global(self) -> bool:
        return self.scope == "*"

    def to_policy_scope(self) -> PolicyScope:
        if self.is_global():
            return PolicyScope(teams=["*"], keys=["*"], models=["*"])
        return PolicyScope(
            teams=self.teams, keys=self.keys, models=self.models, tags=self.tags,
        )


class PolicyConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    policies: dict[str, Policy] = Field(default_factory=dict)
