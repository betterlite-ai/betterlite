import os

os.environ["LITELLM_RUST"] = "false"

import litellm

from .adapter import run

litellm.suppress_debug_info = True

__all__ = ("run",)
