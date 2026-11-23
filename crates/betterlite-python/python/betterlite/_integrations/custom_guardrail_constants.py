import secrets
from typing import Final

from betterlite._settings.env import env_number

# ast-grep-ignore: no-long-names
PRE_CALL_EXECUTED_GUARDRAILS_KEY: Final = "_pre_call_executed_guardrails"
# ast-grep-ignore: no-long-names
LOGS_GUARDRAIL_INFORMATION_MARKER: Final = "_litellm_logs_guardrail_information"
RECORDS_KEY: Final = "standard_logging_guardrail_information"
PRE_CALL_EXECUTED_TOKEN: Final = secrets.token_hex(16)
# ast-grep-ignore: no-long-names
GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS: Final = env_number(
    "GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", 24 * 60 * 60, int
)
DEFAULT_ADVISORY_MESSAGE: Final = (
    "The user's latest message was flagged for {reason} by a content safety guardrail. This "
    "may be a false positive. Use your judgment: respond helpfully if the request is "
    "legitimate, or decline if it is not."
)
TAG_MODE_MESSAGE: Final = (
    "Setting tag-based guardrails is only available in litellm-enterprise. "
    "You must be a premium user to use this feature."
)
NOT_PREMIUM_USER_MESSAGE: Final = (
    "You must be a LiteLLM Enterprise user to use this feature. If you have a license please "
    "set `LITELLM_LICENSE` in your env. Get a 7 day trial key here: "
    "https://www.litellm.ai/enterprise#trial. \nPricing: https://www.litellm.ai/#pricing"
)
NO_SESSION_ROUTING_MESSAGE: Final = (
    "Cannot route sensitive data without a session_id. "
    "Ensure the request includes a session_id in metadata or headers."
)
STREAM_CONTROL_KEYS: Final = frozenset({"stream_holdback_chars"})
