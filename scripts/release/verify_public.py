#!/usr/bin/env python3
"""Validate actual public release/Skill bytes and first installation on a native runner."""
import hashlib
import argparse
import json
import pathlib
import platform
import subprocess
import tempfile
import time
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def verify(skill_only=False):
    config = json.loads((ROOT / 'skills/dt-cli/scripts/distribution.json').read_bytes())
    base = config['publicBaseUrl']
    opener = urllib.request.build_opener(NoRedirect())

    def download(key, limit):
        if not key.startswith(('channels/', 'releases/', 'skills/')) or any(p in ('', '.', '..') for p in key.split('/')):
            raise ValueError('Invalid public object key')
        request = urllib.request.Request(base + key, headers={'Cache-Control': 'no-cache'})
        for attempt in range(16):
            try:
                with opener.open(request, timeout=90) as response:
                    data = response.read(limit + 1)
                    if len(data) > limit:
                        raise ValueError('Oversized public object')
                    return data
            except urllib.error.HTTPError as error:
                if error.code != 404 or attempt == 15:
                    raise
                time.sleep(2)

    stable = json.loads(download('channels/stable.json', 16384))
    raw = download(stable['releaseKey'], 65536)
    if hashlib.sha256(raw).hexdigest() != stable['releaseSha256']:
        raise ValueError('Published release index digest differs')
    release = json.loads(raw)
    commit = release['buildCommit'] if skill_only else subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if release['buildCommit'] != commit or release['version'] != stable['version']:
        raise ValueError('Published release is not this verified source commit')
    if skill_only:
        channel = json.loads(download('channels/skill-stable.json', 16384))
        raw_skill = download(channel['releaseKey'], 16384)
        if hashlib.sha256(raw_skill).hexdigest() != channel['releaseSha256']:
            raise ValueError('Universal Skill index digest differs')
        skill = json.loads(raw_skill)
        if skill['version'] != config['skillVersion'] or skill['version'] != channel['version']:
            raise ValueError('Universal Skill version differs')
        source_commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        if skill['sourceCommit'] != source_commit:
            raise ValueError('Universal Skill was not published from this source commit')
    else:
        skill = next(item for item in release['skills'] if item['key'].endswith('/dt-cli-skill.zip'))
    skill_bytes = download(skill['key'], skill['bytes'])
    if len(skill_bytes) != skill['bytes'] or hashlib.sha256(skill_bytes).hexdigest() != skill['sha256']:
        raise ValueError('Published Skill digest differs')
    with tempfile.TemporaryDirectory(prefix='dt-cli-public-') as temporary:
        temporary = pathlib.Path(temporary).resolve()
        archive = temporary / 'skill.zip'
        archive.write_bytes(skill_bytes)
        extracted = temporary / 'skill'
        with zipfile.ZipFile(archive) as package:
            for item in package.infolist():
                path = pathlib.PurePosixPath(item.filename)
                if not item.filename.startswith('dt-cli/') or path.is_absolute() or '..' in path.parts or item.file_size > 1024 * 1024 or (item.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError('Invalid Skill entries')
            package.extractall(extracted)
        scripts = extracted / 'dt-cli/scripts'
        if json.loads((scripts / 'distribution.json').read_bytes()) != config:
            raise ValueError('Published Skill has a different fixed prefix')
        installation = temporary / 'installation 中文 with spaces'
        argv = ['bash', scripts / 'bootstrap.sh', '--directory', installation] if platform.system() == 'Darwin' else [
            'powershell', '-NonInteractive', '-NoProfile', '-File', scripts / 'bootstrap.ps1', '-Directory', installation]

        def run(args):
            result = subprocess.run([str(a) for a in args], capture_output=True, text=True, encoding='utf-8', timeout=240)
            if result.returncode != 0:
                raise ValueError(f'Public bootstrap failed: {result.stdout} {result.stderr}')
            envelope = json.loads(result.stdout)
            if envelope.get('ok') is not True:
                raise ValueError('Public command returned failure')
            return envelope['data']

        first = run(argv)
        if first['action'] != 'install' or first['version'] != release['version']:
            raise ValueError('Public first installation differs')
        probe = run([first['launcher'], 'version'])
        if probe['buildCommit'] != commit or probe['catalogDigest'] != release['catalogDigest']:
            raise ValueError('Installed public payload provenance differs')
        active = (installation / 'active').read_bytes()
        repeated = run(argv)
        if repeated['updateCheck'] != 'cached' or (installation / 'active').read_bytes() != active:
            raise ValueError('Repeated public bootstrap must use the verified cache without switching')
        checked = run([first['launcher'], 'upgrade', '--online', '--check'])
        if checked['updateAvailable'] is not False or checked['changed'] is not False or (installation / 'active').read_bytes() != active:
            raise ValueError('Actual public online check differs or changed installation')
    print(json.dumps(dict(publicPrefix=base, version=release['version'], skillVersion=config['skillVersion'], buildCommit=commit,
                         os=platform.system(), checks=['public-skill-and-index-digests', 'public-first-install',
                         'public-launcher-provenance', 'public-cached-repeat', 'public-online-check'], iamLoginPerformed=False)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skill-only', action='store_true')
    verify(parser.parse_args().skill_only)
