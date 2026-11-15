import os

import litellm

from .adapter import run

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"
litellm.suppress_debug_info = True
__all__ = ("run",)
