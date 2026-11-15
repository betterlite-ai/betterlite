import re
from collections.abc import Mapping
from types import MappingProxyType
from typing import ClassVar, Literal
from urllib.parse import urlsplit

from pydantic import BaseModel, ConfigDict, Field, SecretStr, field_validator

DEFAULT_PROMPT = (
    "Estimate how many hours it would take an engineer to complete the work in this pull request "
    "without AI assistance. Explain your estimate briefly."
)

_MIN_UPDATE_INTERVAL_MINUTES = 5
_MAX_ESTIMATOR_PROMPT_LENGTH = 20000

_LOGIN_PATTERN = re.compile(r"[A-Za-z0-9_\[\]-]+")
_REPOSITORY_PATTERN = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")
_EMAIL_PATTERN = re.compile(r"[^\s@]+@[^\s@]+\.[^\s@]+")
_NOREPLY_GITHUB_PATTERN = re.compile(r"noreply\.github\.com\Z")


def _normalize_login(value: str) -> str:
    login = value.strip().casefold()
    if _LOGIN_PATTERN.fullmatch(login) is None:
        raise ValueError("Enter a valid GitHub username.")
    return login


class ROISettings(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    github_api_url: str = "https://api.github.com"
    github_token: SecretStr = SecretStr("")
    estimator_key: SecretStr = SecretStr("")
    repos: tuple[str, ...] = ()
    estimator_model: str = ""
    estimator_prompt: str = DEFAULT_PROMPT
    backfill_days: int = Field(default=7, ge=1, le=3650)
    update_interval_minutes: float = Field(default=1440, ge=0, le=43200, allow_inf_nan=False)
    identity_map: Mapping[str, str] = Field(default_factory=dict, validate_default=True)

    @field_validator("update_interval_minutes")
    @classmethod
    def validate_update_interval(cls, value: float) -> float:
        if 0 < value < _MIN_UPDATE_INTERVAL_MINUTES:
            raise ValueError("Choose manual updates (0), or an interval of at least 5 minutes.")
        return value

    @field_validator("github_api_url")
    @classmethod
    def normalize_github_api_url(cls, value: str) -> str:
        normalized = value.strip().rstrip("/")
        if not normalized:
            raise ValueError("A GitHub API URL is required.")
        parsed = urlsplit(normalized)
        if (
            parsed.scheme != "https"
            or not parsed.hostname
            or parsed.username
            or parsed.password
            or parsed.query
            or parsed.fragment
        ):
            raise ValueError("Use an HTTPS GitHub API URL without credentials, query, or fragment.")
        return normalized

    @field_validator("repos")
    @classmethod
    def validate_repositories(cls, values: tuple[str, ...]) -> tuple[str, ...]:
        stripped = tuple(repo.strip().rstrip("/").removesuffix(".git") for repo in values)
        normalized = tuple(
            repo for index, repo in enumerate(stripped) if repo not in stripped[:index]
        )
        invalid_repositories = tuple(
            repo
            for repo in normalized
            if _REPOSITORY_PATTERN.fullmatch(repo) is None
            or any(part in (".", "..") for part in repo.split("/"))
        )
        if invalid_repositories:
            raise ValueError("Repositories must use owner/repo format.")
        return normalized

    @field_validator("estimator_prompt")
    @classmethod
    def validate_estimator_prompt(cls, value: str) -> str:
        normalized = value.strip()
        if not normalized or len(normalized) > _MAX_ESTIMATOR_PROMPT_LENGTH:
            raise ValueError("The estimator prompt must contain between 1 and 20,000 characters.")
        return normalized

    @field_validator("identity_map")
    @classmethod
    def normalize_identity_map(cls, values: Mapping[str, str]) -> Mapping[str, str]:
        normalized_values: dict[str, str] = {}
        for login, address in values.items():
            normalized_address = address.strip().casefold()
            if _EMAIL_PATTERN.fullmatch(normalized_address) is None:
                continue
            if _NOREPLY_GITHUB_PATTERN.search(normalized_address) is not None:
                continue
            normalized_values[_normalize_login(login)] = normalized_address
        if len(normalized_values) != len(values):
            raise ValueError("Each identity needs a GitHub username and a valid gateway email.")
        return MappingProxyType(normalized_values)


class ROISettingsUpdate(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    github_api_url: str | None = None
    github_token: str | None = None
    estimator_key: str | None = None
    repos: tuple[str, ...] | None = None
    estimator_model: str | None = None
    estimator_prompt: str | None = None
    backfill_days: int | None = Field(default=None, ge=1, le=3650)
    update_interval_minutes: float | None = Field(default=None, ge=0, le=43200, allow_inf_nan=False)


class ROISettingsResponse(BaseModel):
    github_api_url: str
    repos: tuple[str, ...]
    estimator_model: str
    estimator_prompt: str
    backfill_days: int
    update_interval_minutes: float
    has_estimator_key: bool
    identity_map: Mapping[str, str]
    has_github_token: bool
    default_prompt: str
    available_models: tuple[str, ...]
    ready: bool


class ROIRepository(BaseModel):
    name: str
    visibility: str
    archived: bool


class ROIRepositoriesResponse(BaseModel):
    repositories: tuple[ROIRepository, ...]
    page: int
    has_more: bool


class ROISyncStatus(BaseModel):
    running: bool
    phase: Literal[
        "idle", "spend", "repositories", "estimates", "complete", "cancelled", "error",
    ]
    stage: str
    done: int
    total: int
    estimated: int
    reused: int
    needs_attention: int
    error: str | None
    started_at: str | None = None
    finished_at: str | None = None
    next_update: str | None = None
    elapsed_seconds: int = 0
    remaining_seconds: int | None = None


class ROIIdentityMapUpdate(BaseModel):
    github_login: str
    email: str | None

    @field_validator("github_login")
    @classmethod
    def normalize_login(cls, value: str) -> str:
        return _normalize_login(value)


__all__ = [
    "DEFAULT_PROMPT", "ROIIdentityMapUpdate", "ROIRepositoriesResponse", "ROIRepository",
    "ROISettings", "ROISettingsResponse", "ROISettingsUpdate", "ROISyncStatus",
    "_normalize_login",
]
