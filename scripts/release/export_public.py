#!/usr/bin/env python3
"""Export buildable committed source without private history or internal acceptance records."""
import argparse
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
FILES = {'.gitignore', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'build.rs'}
PREFIXES = ('src/', 'catalog/', 'tests/', 'examples/', 'scripts/', 'skills/', '.github/workflows/')
PUBLIC_README = 'docs/public/README.md'


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT)


def export(commit, output):
    commit = git('rev-parse', '--verify', f'{commit}^{{commit}}').decode().strip()
    output = output.resolve()
    if output.exists() and any(output.iterdir()):
        raise SystemExit('Export destination must be empty; existing files and Git history are never replaced.')
    selected = []
    for record in git('ls-tree', '-r', '-z', '--full-tree', commit).split(b'\0'):
        if not record:
            continue
        header, raw_path = record.split(b'\t', 1)
        mode, kind, oid = header.decode().split()
        path = raw_path.decode('utf-8')
        if path not in FILES and not path.startswith(PREFIXES) and path != PUBLIC_README:
            continue
        if kind != 'blob' or mode not in ('100644', '100755'):
            raise SystemExit(f'Non-regular public input: {path}')
        destination = 'README.md' if path == PUBLIC_README else path
        selected.append((destination, mode, git('cat-file', 'blob', oid)))
    if not any(path == 'README.md' for path, _, _ in selected):
        raise SystemExit('Committed public README is missing.')
    output.mkdir(parents=True, exist_ok=True)
    digests = {}
    for path, mode, content in selected:
        target = output / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
        target.chmod(int(mode[-3:], 8))
        digests[path] = hashlib.sha256(content).hexdigest()
    manifest = {'schemaVersion': 1, 'sourceCommit': commit, 'files': digests}
    (output / 'source-export.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'sourceCommit': commit, 'directory': str(output), 'filesExported': len(selected)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--commit', default='HEAD')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    export(args.commit, args.output)


if __name__ == '__main__':
    main()
