#!/bin/sh
# Print a models/runtimes.toml for a llama.cpp release tag using the SHA-256
# digests GitHub publishes for release assets. Review, then replace the file.
#   scripts/update-llama-pin.sh b11396 > models/runtimes.toml
set -eu
exec python3 "$(dirname "$0")/update_llama_pin.py" "${1:?usage: update-llama-pin.sh <tag>}"
