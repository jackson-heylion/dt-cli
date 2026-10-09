#!/bin/bash
# Prepare the verified native launcher and import into the explicitly selected Agent directory.
set -eu
scripts=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
temporary=$(mktemp)
trap 'rm -f -- "$temporary"' EXIT
if [ "${1:-}" = --installation-directory ]; then
    installation=$2
    shift 2
    bash "$scripts/bootstrap.sh" --directory "$installation" > "$temporary"
else
    bash "$scripts/bootstrap.sh" > "$temporary"
fi
launcher=$(/usr/bin/plutil -extract data.launcher raw -o - "$temporary")
rm -f -- "$temporary"
exec "$launcher" skill install "$@"
