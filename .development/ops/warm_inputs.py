import argparse
import hashlib
import subprocess
from pathlib import Path

INPUTS = (
    "tools/oracle/helpers",
    "tools/oracle/cases",
    "tools/oracle/litellm/uv.lock",
    "tools/oracle/litellm/pyproject.toml",
    "tools/oracle/litellm/.python-version",
    ".development/litellm.yaml",
    "tools/oracle/src/helpers/answers.rs",
)


def fingerprint(root: Path, revision: str) -> str:
    listing = subprocess.check_output(
        ["git", "-C", str(root), "ls-tree", "-rz", "--full-tree", revision, "--", *INPUTS]
    )
    return hashlib.sha256(listing).hexdigest()


class Arguments(argparse.Namespace):
    root: Path
    revision: str


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Fingerprint the inputs that can add or invalidate cached oracle answers."
    )
    parser.add_argument("root", type=Path)
    parser.add_argument("revision")
    args = parser.parse_args(namespace=Arguments())
    print(fingerprint(args.root, args.revision))
