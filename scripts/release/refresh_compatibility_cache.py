#!/usr/bin/env python3
"""Refresh only the immutable public fixtures used by upgrade verification."""
import os
import sys

from publish_qiniu import QiniuStore, configuration

# 历史升级验收依赖的固定对象；不包含 stable，不接受任意对象或目录。
COMPATIBILITY_KEYS = (
    'releases/0.4.1/release.json',
    'releases/0.4.1/dt-cli-macos-arm64.zip',
    'releases/0.4.1/dt-cli-windows-x64.zip',
    'skills/0.4.4/dt-cli-skill.zip',
    'releases/0.5.5/native-release.json',
    'releases/0.5.5/dt-cli-macos-arm64.zip',
    'releases/0.5.5/dt-cli-macos-x64.zip',
    'releases/0.5.5/dt-cli-windows-x64.zip',
    'skills/0.5.5/release.json',
    'skills/0.5.5/dt-cli-skill.zip',
)


def refresh(store):
    # 只提交 CDN 失效请求；由后续原生验收检查原路径、长度和摘要。
    store.refresh_many(COMPATIBILITY_KEYS)


def main():
    try:
        base, bucket, region = (os.environ[name] for name in (
            'QINIU_PUBLIC_BASE_URL', 'QINIU_BUCKET', 'QINIU_REGION'))
        prefix = configuration(base, bucket, region)
        refresh(QiniuStore(bucket, region, base, prefix))
    except Exception:
        # SDK 异常可能含签名 URL，不输出原始异常或凭据。
        print('COMPATIBILITY_CACHE_REFRESH_FAILED: inspect CDN refresh permission and configuration.', file=sys.stderr)
        raise SystemExit(1)


if __name__ == '__main__':
    main()
