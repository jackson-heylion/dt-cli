#!/usr/bin/env python3
"""Validate every native target and retain the two-target index for older clients."""
import argparse
import hashlib
import json
import pathlib
import re
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
MAX_ZIP = 256 * 1024 * 1024
COMPATIBILITY = dict(bootstrapSchema=1, profileFormat=1, credentialFormat=1,
                     installerSchema=1, launcherSchema=1, minimumSkillVersion='0.4.1',
                     maximumSkillVersionExclusive='0.6.0')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def version(value):
    if not isinstance(value, str) or not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', value):
        raise ValueError('Invalid fixed version')
    return tuple(map(int, value.split('.')))


def key(value):
    if not isinstance(value, str) or not re.fullmatch(r'(releases|skills)/[a-zA-Z0-9./_-]+', value) or any(
            part in ('', '.', '..') for part in value.split('/')):
        raise ValueError('Invalid relative distribution key')
    return value


def read_json(path):
    if path.stat().st_size > 65536:
        raise ValueError('Oversized metadata')
    return json.loads(path.read_bytes())


def native(folder, name, target):
    archive = folder / f'dt-cli-{name}.zip'
    if not 0 < archive.stat().st_size <= MAX_ZIP:
        raise ValueError('Invalid ZIP size')
    data = archive.read_bytes()
    manifest = read_json(archive.with_suffix('.manifest.json'))
    evidence = read_json(folder / f'dt-cli-{name}.native-check.json')
    with zipfile.ZipFile(archive) as package:
        entries = package.infolist()
        if len(entries) != 4 or {item.filename for item in entries} != {
                target['binary'], 'manifest.json', 'install.sh', 'install.ps1'} or any(
                item.file_size > MAX_ZIP or (item.external_attr >> 16) & 0o170000 == 0o120000 for item in entries):
            raise ValueError('Invalid ZIP entries')
        if json.loads(package.read('manifest.json')) != manifest or digest(package.read(target['binary'])) != manifest['sha256']:
            raise ValueError('ZIP payload differs from manifest')
    version(manifest['version'])
    if manifest['releaseType'] != 'release' or manifest['nativeProbe'] != 'performed' or manifest['localDevelopment'] is not False:
        raise ValueError('Only native stable releases can be distributed')
    if not re.fullmatch(r'[0-9a-f]{40}', manifest['buildCommit']) or not re.fullmatch(r'[0-9a-f]{64}', manifest['catalogDigest']):
        raise ValueError('Dirty or invalid source provenance')
    for field, expected in {'buildTarget': target['target'], **{k: target[k] for k in ('os', 'architecture', 'binary')},
                            'manifestSchemaVersion': 1, 'profileFormat': 1, 'credentialFormat': 1,
                            'minimumInstallerSchema': 1, 'minimumLauncherSchema': 1}.items():
        if manifest[field] != expected:
            raise ValueError(f'Incompatible native manifest: {field}')
    for field in ('version', 'buildCommit', 'catalogDigest', 'sourceVersion'):
        if evidence[field] != manifest[field]:
            raise ValueError('Native evidence does not describe this manifest')
    if evidence['target'] != target['target'] or evidence['archiveSha256'] != digest(data) or evidence['nativeProbe'] != 'performed':
        raise ValueError('Native evidence does not describe this ZIP')
    if archive.with_suffix('.zip.sha256').read_text().split()[0] != digest(data):
        raise ValueError('ZIP checksum sidecar differs')
    return manifest, data


def assemble(artifacts, output, sequence):
    if isinstance(sequence, bool) or not isinstance(sequence, int) or not 0 < sequence <= 9007199254740991:
        raise ValueError('Release sequence must be positive')
    matrix = read_json(ROOT / 'catalog/release-targets.json')['targets']
    packages, manifests, copies = [], [], {}
    folders = []
    for target in matrix:
        name = target['name']
        candidates = list(artifacts.rglob(f'dt-cli-{name}.zip'))
        if len(candidates) != 1:
            raise ValueError(f'Require exactly one artifact for {name}')
        folder = candidates[0].parent
        folders.append(folder)
        manifest, data = native(folder, name, target)
        manifests.append(manifest)
        prefix = f'releases/{manifest["version"]}/'
        filename = f'dt-cli-{name}.zip'
        copies[prefix + filename] = data
        for suffix in ('.manifest.json', '.zip.sha256', '.native-check.json'):
            source = folder / (f'dt-cli-{name}' + suffix)
            copies[prefix + source.name] = source.read_bytes()
        packages.append(dict(target=target['target'], os=target['os'], architecture=target['architecture'],
                             key=prefix + filename, sha256=digest(data), bytes=len(data),
                             binarySha256=manifest['sha256']))
    for field in ('version', 'buildCommit', 'catalogDigest', 'catalogVersion', 'sourceVersion'):
        if len({m[field] for m in manifests}) != 1:
            raise ValueError(f'Native targets disagree: {field}')
    config = read_json(ROOT / 'skills/dt-cli/scripts/distribution.json')
    skill_version = config['skillVersion']
    if not version(COMPATIBILITY['minimumSkillVersion']) <= version(skill_version) < version(COMPATIBILITY['maximumSkillVersionExclusive']):
        raise ValueError('Packaged Skill is outside the release compatibility range')
    skills = []
    for filename in ('dt-cli-skill.zip',):
        contents = [(folder / filename).read_bytes() for folder in folders]
        if any(content != contents[0] for content in contents[1:]):
            raise ValueError('Native jobs packaged different Skills')
        with zipfile.ZipFile(folders[0] / filename) as package:
            config_bytes = package.read('dt-cli/scripts/distribution.json')
            if json.loads(config_bytes) != config:
                raise ValueError('Skill has a different distribution configuration')
        object_key = f'skills/{skill_version}/{filename}'
        copies[object_key] = contents[0]
        skills.append(dict(key=object_key, sha256=digest(contents[0]), bytes=len(contents[0])))
    release = dict(schemaVersion=1, version=manifests[0]['version'], buildCommit=manifests[0]['buildCommit'],
                   catalogDigest=manifests[0]['catalogDigest'], compatibility=COMPATIBILITY,
                   packages=packages, skills=skills)
    release_bytes = (json.dumps(release, indent=2) + '\n').encode()
    release_key = f'releases/{release["version"]}/native-release.json'
    copies[release_key] = release_bytes
    stable = dict(schemaVersion=1, sequence=sequence, version=release['version'],
                  releaseKey=release_key, releaseSha256=digest(release_bytes))
    copies['channels/native-stable.json'] = (json.dumps(stable, indent=2) + '\n').encode()
    # CLI/Skill 0.4.1 readers require exactly ARM Mac + Windows and this fixed index path.
    legacy = dict(release, packages=[p for p in packages if p['target'] != 'x86_64-apple-darwin'])
    legacy_bytes = (json.dumps(legacy, indent=2) + '\n').encode()
    legacy_key = f'releases/{release["version"]}/release.json'
    copies[legacy_key] = legacy_bytes
    copies['channels/stable.json'] = (json.dumps(dict(stable, releaseKey=legacy_key,
        releaseSha256=digest(legacy_bytes)), indent=2) + '\n').encode()
    if output.exists() and any(output.iterdir()):
        raise ValueError('Distribution output must be empty')
    for object_key, data in copies.items():
        path = output / object_key
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return stable


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--sequence', type=int, required=True)
    args = parser.parse_args()
    print(json.dumps(assemble(args.artifacts, args.output, args.sequence)))


if __name__ == '__main__':
    main()
