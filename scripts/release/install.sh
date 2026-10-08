#!/bin/sh
# Verify the downloaded ZIP and extracted executable before running the native installer.
set -eu
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
package= expected= pending=
for argument in "$@"; do
    if [ -n "$pending" ]; then
        case "$pending" in package) package=$argument;; sha256) expected=$argument;; esac
        pending=
    else
        case "$argument" in
            --package) pending=package;; --sha256) pending=sha256;;
            --package=*) package=${argument#*=};; --sha256=*) expected=${argument#*=};;
            --signer|--directory|--format) pending=ignored;;
        esac
    fi
done
if [ -z "$package" ] || [ ${#expected} -ne 64 ]; then
    printf '%s\n' '请提供 --package 和组织可信入口公布的 ZIP --sha256。' >&2; exit 2
fi
actual=$(shasum -a 256 < "$package" | cut -c1-64)
[ "$actual" = "$expected" ] || { printf '%s\n' 'ZIP 摘要不一致。' >&2; exit 1; }
binary_digest=$(/usr/bin/unzip -p "$package" manifest.json | /usr/bin/plutil -extract sha256 raw -o - -)
actual=$(shasum -a 256 < "$package_dir/dt-cli" | cut -c1-64)
[ "$actual" = "$binary_digest" ] || { printf '%s\n' '解压程序与已核对的 ZIP 不一致。' >&2; exit 1; }
exec "$package_dir/dt-cli" install "$@"
