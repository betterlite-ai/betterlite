import os

os.environ["LITELLM_RUST"] = "false"

import litellm

from .handlers import run as run

_ = litellm
