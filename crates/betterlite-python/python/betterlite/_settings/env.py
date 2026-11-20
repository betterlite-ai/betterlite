import os
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Final, TypeVar

from dotenv import load_dotenv

_Number = TypeVar("_Number", int, float)

_DEV_MODE: Final = "DEV"
_HOT_RELOAD_VARIABLE: Final = "LITELLM_DEV_ENV_HOT_RELOAD"
_TTL_VARIABLE: Final = "LITELLM_ANTHROPIC_PROMPT_CACHING_TTL"
_TRUE_VALUES: Final = frozenset({"true", "1", "yes", "on"})
_FALSE_VALUES: Final = frozenset({"false", "0", "no", "off"})
_CACHING_TTLS: Final = frozenset({"1h", "5m"})
_COST_MAP_URL: Final = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "model_prices_and_context_window.json"
)
_BLOG_URL: Final = "https://docs.litellm.ai/blog/rss.xml"
_BETA_HEADERS_URL: Final = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "litellm/anthropic_beta_headers_config.json"
)
_PRESETS_URL: Final = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "litellm/proxy/public_endpoints/autorouter_presets.json"
)


def _env_text(variable: str) -> str | None:
    raw = os.environ.get(variable)
    if raw is None:
        return None
    text = raw.strip()
    if text == "":
        return None
    return text


def env_bool(variable: str) -> bool:
    text = _env_text(variable)
    if text is None:
        return False
    normalized = text.lower()
    if normalized in _TRUE_VALUES:
        return True
    if normalized in _FALSE_VALUES:
        return False
    raise ValueError(f"{variable} must be a boolean, got {text!r}")


def env_number(variable: str, default: _Number, convert: Callable[[str], _Number]) -> _Number:
    text = _env_text(variable)
    if text is None:
        return default
    try:
        return convert(text)
    except ValueError as error:
        raise ValueError(f"{variable} must be a number, got {text!r}") from error


def env_str(variable: str, default: str) -> str:
    text = _env_text(variable)
    if text is None:
        return default
    return text


@dataclass(frozen=True)
class EnvSettings:
    mode: str
    max_tokens: int
    drop_params: bool
    modify_params: bool
    chat_url: bool
    route_responses: bool
    gemini_setup: bool
    prompt_caching: bool
    ttl_env: str | None
    ttl: str | None
    default_soft_budget: float
    model_cost_map_url: str
    blog_posts_url: str
    anthropic_beta_headers_url: str
    autorouter_presets_url: str


def load_env_file() -> None:
    if env_str("LITELLM_MODE", _DEV_MODE) != _DEV_MODE:
        return
    load_dotenv(dotenv_path=Path(".env"), override=env_bool(_HOT_RELOAD_VARIABLE))


def env_settings() -> EnvSettings:
    load_env_file()
    ttl_env = _env_text(_TTL_VARIABLE)
    ttl = ttl_env if ttl_env in _CACHING_TTLS else None
    return EnvSettings(
        mode=env_str("LITELLM_MODE", _DEV_MODE),
        max_tokens=env_number("DEFAULT_MAX_TOKENS", 4096, int),
        drop_params=env_bool("LITELLM_DROP_PARAMS"),
        modify_params=env_bool("LITELLM_MODIFY_PARAMS"),
        chat_url=env_bool("LITELLM_USE_CHAT_COMPLETIONS_URL_FOR_ANTHROPIC_MESSAGES"),
        route_responses=env_bool("LITELLM_ROUTE_ALL_CHAT_OPENAI_TO_RESPONSES"),
        gemini_setup=env_bool("LITELLM_GEMINI_LIVE_DEFER_SETUP"),
        prompt_caching=env_bool("LITELLM_ENABLE_ANTHROPIC_PROMPT_CACHING"),
        ttl_env=ttl_env,
        ttl=ttl,
        default_soft_budget=env_number("DEFAULT_SOFT_BUDGET", 50.0, float),
        model_cost_map_url=env_str("LITELLM_MODEL_COST_MAP_URL", _COST_MAP_URL),
        blog_posts_url=env_str("LITELLM_BLOG_POSTS_URL", _BLOG_URL),
        anthropic_beta_headers_url=env_str("LITELLM_ANTHROPIC_BETA_HEADERS_URL", _BETA_HEADERS_URL),
        autorouter_presets_url=env_str("LITELLM_AUTOROUTER_PRESETS_URL", _PRESETS_URL),
    )


__all__ = [
    "env_settings",
]
