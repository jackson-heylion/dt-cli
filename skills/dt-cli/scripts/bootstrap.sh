#!/bin/bash
# First installation uses macOS system tools; subsequent updates use the verified native CLI.
set -eu
umask 077
scripts=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root="$HOME/Library/Application Support/com.datousoft.dt-cli/installation"
refresh=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --directory) root=$2; shift 2;;
        --refresh) refresh=true; shift;;
        *) printf '%s\n' 'BOOTSTRAP_ARGUMENT_INVALID' >&2; exit 2;;
    esac
done
[ "$(uname -s)" = Darwin ] || { printf '%s\n' 'PLATFORM_MISMATCH' >&2; exit 1; }
host_arch=$(uname -m)
case "$host_arch" in arm64|x86_64) ;; *) printf '%s\n' 'PLATFORM_MISMATCH' >&2; exit 1;; esac
case "$root" in /*) ;; *) printf '%s\n' 'BOOTSTRAP_DIRECTORY_INVALID' >&2; exit 1;; esac
cursor=$root
while [ "$cursor" != / ]; do
    [ ! -L "$cursor" ] || { printf '%s\n' 'BOOTSTRAP_DIRECTORY_INVALID' >&2; exit 1; }
    cursor=$(dirname -- "$cursor")
done
temp=$(mktemp -d)
finish() {
    status=$?
    rm -rf -- "$temp"
    if [ "$status" != 0 ]; then
        printf '%s\n' '{"ok":false,"data":null,"error":{"code":"BOOTSTRAP_FAILED","message":"CLI preparation failed; inspect the fixed distribution entry and installed program."}}'
    fi
}
trap finish EXIT
validate() { /usr/bin/osascript -l JavaScript "$scripts/validate-macos.js" "$@"; }
get() {
    curl --silent --show-error --fail --proto '=https' --connect-timeout 5 --max-time 90 \
        --max-filesize "$3" --header 'Cache-Control: no-cache' --output "$2" "$1"
    [ "$(stat -f %z "$2")" -le "$3" ]
}
hash() { /usr/bin/shasum -a 256 < "$1" | /usr/bin/cut -c1-64; }
launcher="$root/bin/dt-cli"
minimum=$(/usr/bin/plutil -extract minimumCliVersion raw -o - "$scripts/distribution.json")
action=existing
update=not-checked
if [ -x "$launcher" ]; then
    "$launcher" version > "$temp/probe.json"
    validate probe "$temp/probe.json" "$minimum" "$host_arch" > /dev/null || action=upgrade
    [ "$(/usr/bin/plutil -extract data.cliVersion raw -o - "$temp/probe.json")" != 0.4.0 ] || action=upgrade
else
    action=install
fi
if [ "$action" = upgrade ]; then
    # Prefer the installed updater, including its cached sequence and no-downgrade checks.
    if "$launcher" upgrade --online --minimum-version "$minimum" --directory "$root" > "$temp/update.json"; then
        "$launcher" version > "$temp/probe.json"
        if validate probe "$temp/probe.json" "$minimum" "$host_arch" > /dev/null; then action=existing; fi
    fi
fi
if [ "$action" = install ] || [ "$action" = upgrade ]; then
    validate config "$scripts/distribution.json" > "$temp/config.json"
    base=$(/usr/bin/plutil -extract publicBaseUrl raw -o - "$temp/config.json")
    get "${base}channels/native-stable.json" "$temp/stable.json" 16384
    key=$(/usr/bin/plutil -extract releaseKey raw -o - "$temp/stable.json")
    # Restrict metadata before even sending the next request.
    [[ "$key" =~ ^releases/[0-9]+\.[0-9]+\.[0-9]+/native-release\.json$ ]] || exit 1
    expected=$(/usr/bin/plutil -extract releaseSha256 raw -o - "$temp/stable.json")
    get "$base$key" "$temp/release.json" 65536
    [ "$(hash "$temp/release.json")" = "$expected" ] || exit 1
    validate plan "$scripts/distribution.json" "$temp/stable.json" "$temp/release.json" "$host_arch" > "$temp/plan.json"
    key=$(/usr/bin/plutil -extract package.key raw -o - "$temp/plan.json")
    expected=$(/usr/bin/plutil -extract package.sha256 raw -o - "$temp/plan.json")
    size=$(/usr/bin/plutil -extract package.bytes raw -o - "$temp/plan.json")
    get "$base$key" "$temp/package.zip" "$size"
    [ "$(stat -f %z "$temp/package.zip")" = "$size" ] && [ "$(hash "$temp/package.zip")" = "$expected" ] || exit 1
    /usr/bin/unzip -Z1 "$temp/package.zip" > "$temp/entries"
    [ "$(wc -l < "$temp/entries" | tr -d ' ')" = 4 ] || exit 1
    for name in dt-cli manifest.json install.sh install.ps1; do
        [ "$(awk -v name="$name" '$0 == name {n++} END {print n+0}' "$temp/entries")" = 1 ] || exit 1
    done
    # Copy only two fixed entries; links and paths from the archive are never materialized.
    (ulimit -f 128; /usr/bin/unzip -p "$temp/package.zip" manifest.json > "$temp/manifest.json")
    binary_hash=$(validate manifest "$temp/manifest.json" "$temp/plan.json")
    (ulimit -f 524288; /usr/bin/unzip -p "$temp/package.zip" dt-cli > "$temp/dt-cli")
    [ "$(hash "$temp/dt-cli")" = "$binary_hash" ] || exit 1
    chmod 755 "$temp/dt-cli"
    if [ "$action" = install ]; then
        "$temp/dt-cli" install --package "$temp/package.zip" --sha256 "$expected" --directory "$root" > "$temp/install.json"
    else
        "$temp/dt-cli" upgrade --package "$temp/package.zip" --sha256 "$expected" --directory "$root" > "$temp/install.json"
    fi
    update=online
fi
# A 0.4.0 installation can be upgraded by the new package if it lacks the remote command.
"$launcher" version > "$temp/probe.json"
installed=$(validate probe "$temp/probe.json" "$minimum" "$host_arch")
if [ "$installed" != 0.4.0 ]; then
    options=(upgrade --online --minimum-version "$minimum" --directory "$root")
    [ "$refresh" = true ] || options+=(--cached)
    if "$launcher" "${options[@]}" > "$temp/update.json"; then
        update=$(/usr/bin/plutil -extract data.updateCheck raw -o - "$temp/update.json")
        changed=$(/usr/bin/plutil -extract data.changed raw -o - "$temp/update.json")
        [ "$changed" = false ] || action=upgrade
    else
        update=failed-compatible-existing
    fi
fi
"$launcher" version > "$temp/probe.json"
validate probe "$temp/probe.json" "$minimum" "$host_arch" > /dev/null
validate result "$temp/probe.json" "$launcher" "$action" "$update"
