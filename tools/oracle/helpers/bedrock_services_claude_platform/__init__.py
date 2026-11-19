import litellm

from .operations import run

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

__all__ = ["run"]
