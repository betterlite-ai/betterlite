#!/bin/bash
set -euo pipefail

betterlite-db-admin migrate "$@"

echo "Migration script ran successfully!"
