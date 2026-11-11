import os

os.environ["LITELLM_RUST"] = "false"

import litellm

from .handlers import run

__all__ = ["run"]

_ = litellm
