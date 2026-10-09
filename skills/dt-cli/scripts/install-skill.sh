#!/bin/bash
# Prepare the verified native launcher and import into the explicitly selected Agent directory.
set -eu
scripts=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
temporary=$(mktemp)
trap 'rm -f -- "$temporary"' EXIT
bash "$scripts/bootstrap.sh" > "$temporary"
launcher=$(/usr/bin/plutil -extract data.launcher raw -o - "$temporary")
rm -f -- "$temporary"
exec "$launcher" skill install "$@"
