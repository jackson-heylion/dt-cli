#!/usr/bin/env python3
"""Upload immutable release objects, verify public bytes, then switch stable last."""
import argparse
import hashlib
import json
import os
import pathlib
import re
import sys
import time
from urllib.parse import urlparse

ROOT = pathlib.Path(__file__).resolve().parents[2]
REGIONS = {'z0': 'upload.qiniup.com', 'cn-east-2': 'upload-cn-east-2.qiniup.com',
           'z1': 'upload-z1.qiniup.com', 'z2': 'upload-z2.qiniup.com',
           'na0': 'upload-na0.qiniup.com', 'as0': 'upload-as0.qiniup.com',
           'ap-southeast-2': 'upload-ap-southeast-2.qiniup.com',
           'ap-southeast-3': 'upload-ap-southeast-3.qiniup.com'}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def configuration(base, bucket, region):
    url = urlparse(base)
    if url.scheme != 'https' or not url.hostname or url.username or url.password or url.query or url.fragment or not re.fullmatch(r'/([a-zA-Z0-9_-]+/)*', url.path):
        raise ValueError('Require a fixed HTTPS distribution prefix ending in /')
    if not re.fullmatch(r'[a-zA-Z0-9_-]+', bucket) or region not in REGIONS:
        raise ValueError('Invalid bucket or unsupported Qiniu region code')
    saved = json.loads((ROOT / 'skills/dt-cli/scripts/distribution.json').read_bytes())
    if saved['publicBaseUrl'] != base:
        raise ValueError('Published prefix must equal the configuration embedded in CLI and Skill')
    return url.path.lstrip('/')


class QiniuStore:
    def __init__(self, bucket, region, base, prefix):
        import qiniu
        import requests
        self.qiniu = qiniu
        self.requests = requests
        self.bucket = bucket
        self.base = base
        self.prefix = prefix
        self.auth = qiniu.Auth(os.environ['QINIU_ACCESS_KEY'], os.environ['QINIU_SECRET_KEY'])
        from qiniu.http.region import Region
        self.regions = [Region.from_region_id(region, preferred_scheme='https')]
        qiniu.set_default(default_zone=qiniu.Zone(scheme='https'), connection_timeout=90)

    def read(self, key, maximum):
        with self.requests.get(self.base + key, headers={'Cache-Control': 'no-cache'},
                               timeout=(5, 90), allow_redirects=False, stream=True) as response:
            if response.status_code == 404:
                return None
            if response.status_code != 200:
                raise ValueError('Public readback failed')
            content = bytearray()
            for chunk in response.iter_content(65536):
                content.extend(chunk)
                if len(content) > maximum:
                    raise ValueError('Public object exceeds expected size')
            return bytes(content)

    def upload(self, key, path, immutable):
        object_key = self.prefix + key
        token = self.auth.upload_token(self.bucket, object_key, 300, policy={'insertOnly': 1 if immutable else 0})
        mime = 'application/json' if key.endswith('.json') else 'application/zip' if key.endswith('.zip') else 'text/plain'
        result, info = self.qiniu.put_file_v2(token, object_key, str(path), mime_type=mime, check_crc=True,
                                              bucket_name=self.bucket, regions=self.regions)
        if result is None or result.get('key') != object_key:
            # A previous interrupted run may already have inserted this immutable key.
            if immutable and getattr(info, 'status_code', None) == 614:
                return
            raise ValueError('Qiniu upload failed; inspect the bucket and scoped upload permission')


def publish(tree, store, wait_for_readback=False):
    stable_path = tree / 'channels/stable.json'
    stable_bytes = stable_path.read_bytes()
    stable = json.loads(stable_bytes)
    release_key = stable['releaseKey']
    if stable['schemaVersion'] != 1 or not 0 < stable['sequence'] <= 9007199254740991 or stable['sequence'] != int(stable['sequence']):
        raise ValueError('Invalid stable sequence')
    if release_key != f'releases/{stable["version"]}/release.json' or sha((tree / release_key).read_bytes()) != stable['releaseSha256']:
        raise ValueError('Stable index does not match release index')
    release = json.loads((tree / release_key).read_bytes())
    expected = {obj['key']: obj for obj in release['packages'] + release['skills']}
    if len(release['packages']) != 2 or release['version'] != stable['version']:
        raise ValueError('Incomplete release')
    for key, obj in expected.items():
        data = (tree / key).read_bytes()
        if sha(data) != obj['sha256'] or len(data) != obj['bytes']:
            raise ValueError('Distribution file differs from release index')

    def guard():
        previous = store.read('channels/stable.json', 16384)
        if previous is not None:
            old = json.loads(previous)
            old_version = tuple(map(int, old['version'].split('.')))
            new_version = tuple(map(int, stable['version'].split('.')))
            if old['sequence'] > stable['sequence'] or old_version > new_version or old['sequence'] == stable['sequence'] and previous != stable_bytes:
                raise ValueError('Stable sequence/version rollback or sequence reuse blocked')
        return previous

    previous = guard()
    objects = sorted(path for path in tree.rglob('*') if path.is_file() and path not in (stable_path, tree / release_key))
    objects.append(tree / release_key)
    for path in objects:
        relative = path.relative_to(tree).as_posix()
        if path.is_symlink() or not re.fullmatch(r'(releases|skills)/[a-zA-Z0-9./_-]+', relative) or any(p in ('', '.', '..') for p in relative.split('/')):
            raise ValueError('Invalid distribution object')
        data = path.read_bytes()
        actual = store.read(relative, len(data))
        uploaded = actual is None
        if uploaded:
            store.upload(relative, path, immutable=True)
            actual = store.read(relative, len(data))
            if wait_for_readback:
                for _ in range(15):
                    if actual == data:
                        break
                    time.sleep(2)
                    actual = store.read(relative, len(data))
        if actual != data:
            raise ValueError('Immutable key has different content or public readback is stale: ' + relative)
        if wait_for_readback:
            print(json.dumps(dict(key=relative, verified=True, uploaded=uploaded)), flush=True)
    # Detect a competing publisher or stale stable response immediately before switching.
    if guard() != previous:
        raise ValueError('Stable changed during this publish; retry against the current channel')
    if previous != stable_bytes:
        store.upload('channels/stable.json', stable_path, immutable=False)
    actual_stable = store.read('channels/stable.json', 16384)
    if wait_for_readback:
        for _ in range(15):
            if actual_stable == stable_bytes:
                break
            time.sleep(2)
            actual_stable = store.read('channels/stable.json', 16384)
    if actual_stable != stable_bytes:
        raise ValueError('Stable public readback differs; verify origin/CDN cache configuration')
    return dict(published=True, version=stable['version'], sequence=stable['sequence'], objectsVerified=len(objects), stableVerified=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tree', type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        base, bucket, region = (os.environ[name] for name in ('QINIU_PUBLIC_BASE_URL', 'QINIU_BUCKET', 'QINIU_REGION'))
        prefix = configuration(base, bucket, region)
        print(json.dumps(publish(args.tree, QiniuStore(bucket, region, base, prefix), wait_for_readback=True)))
    except Exception as error:
        # SDK errors may contain signed request URLs; never echo raw transport exceptions.
        if isinstance(error, ValueError) and type(error) is ValueError:
            print(str(error), file=sys.stderr)
        print('QINIU_PUBLISH_FAILED: inspect public configuration, immutable versions and readback availability.', file=sys.stderr)
        raise SystemExit(1)


if __name__ == '__main__':
    main()
