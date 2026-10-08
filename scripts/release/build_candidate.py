#!/usr/bin/env python3
"""One maintainer command for cross/native build + candidate packaging; no signing or publishing."""
import argparse
import pathlib
import platform
import subprocess
from package import TARGETS, package


def main():
    root = pathlib.Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    architecture = {'amd64': 'x86_64', 'aarch64': 'arm64'}.get(platform.machine().lower(), platform.machine().lower())
    native = next((target for target, host in TARGETS.items() if host == (platform.system(), architecture)), None)
    parser.add_argument('--target', choices=tuple(TARGETS), default=native, required=native is None)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    # Rust target/toolchain setup remains a maintainer prerequisite, not an employee runtime dependency.
    subprocess.run(['cargo', 'build', '--release', '--locked', '--target', args.target], cwd=root, check=True)
    folder = root/'target'/args.target/'release'
    import json
    info = json.loads((folder/'dt-cli.build.json').read_text(encoding='utf-8'))
    os_name, arch = TARGETS[args.target]
    filename = f'dt-cli-{info["cliVersion"]}-candidate-{os_name.lower()}-{arch}.zip'
    package(folder/('dt-cli.exe' if os_name == 'Windows' else 'dt-cli'), args.output or root/'output'/'releases'/filename, 'candidate')


if __name__ == '__main__':
    main()
