"""Controlled LiteLLM startup hook for native-feature oracle cases."""

from litellm.proxy import proxy_server


def set_premium_user() -> None:
    """Enable MIT handlers before LiteLLM evaluates its premium flag."""
    proxy_server.premium_user = True
