#!/usr/bin/env python3
"""Export a portable skill without installing it into any Agent's directories."""
import argparse
import hashlib
import json
import pathlib
import re
import os
import tempfile
import zipfile

import sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from skill_constants import (RESOURCE_DIRECTORIES, ROOT_FILES, EXCLUDED_PARTS, EXCLUDED_SUFFIXES,
                             MAX_PACKAGE_BYTES, MAX_FILE_BYTES, MAX_FILES, PACKAGE_TIMESTAMP, PORTABLE_FILE_MODE)


def resource_files(source):
    files = [source / 'SKILL.md', source / 'LICENSE']
    for name in ROOT_FILES[2:]:
        if (source / name).exists():
            files.append(source / name)
    for directory in RESOURCE_DIRECTORIES:
        root = source / directory
        if root.is_symlink():
            raise SystemExit('不分发符号链接目录。')
        for path in sorted(root.rglob('*')):
            relative = path.relative_to(source)
            if path.is_symlink():
                raise SystemExit('不分发符号链接。')
            if any(part in EXCLUDED_PARTS for part in relative.parts) or path.suffix in EXCLUDED_SUFFIXES:
                continue
            if path.is_file():
                if path.name.startswith('.env') or 'credential' in path.name.lower():
                    raise SystemExit('个人环境或凭证文件不能进入技能包。')
                files.append(path)
    for name in ('openai.yaml', 'qwenwork.json'):
        path = source / 'agents' / name
        if path.exists():
            files.append(path)
    if (source / 'agents/qwenwork.json').exists() and not (source / '.skill-metadata.yaml').exists():
        raise SystemExit('千问办公包缺少推荐任务元数据。')
    return sorted(files, key=lambda path: path.relative_to(source).as_posix())


def validate_references(source, files):
    included = {path.relative_to(source).as_posix() for path in files}
    for path in files:
        if path.suffix != '.md':
            continue
        for target in re.findall(r'\]\(([^\s)]+)(?:\s+"[^"]*")?\)', path.read_text(encoding='utf-8')):
            target = target.strip('<>').split('#', 1)[0]
            if not target or re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:', target):
                continue
            candidate = (path.parent / target).resolve()
            try:
                relative = candidate.relative_to(source.resolve()).as_posix()
            except ValueError:
                raise SystemExit('技能引用必须包含在同一个包中：' + target)
            if relative not in included:
                raise SystemExit('技能引用文件缺失或未分发：' + target)


def entry_bytes(source):
    text = (source / 'SKILL.md').read_text(encoding='utf-8')
    localized_path = source / 'agents' / 'qwenwork.json'
    if localized_path.exists():
        localized = json.loads(localized_path.read_text(encoding='utf-8'))
        required = ('name_en', 'name_zh', 'description', 'description_en', 'description_zh',
                    'argument-hint', 'argument-hint-en', 'argument-hint-zh', 'user-invocable')
        if set(localized) != set(required) or localized['user-invocable'] is not True or any(
                not isinstance(localized[key], str) or not localized[key].strip()
                for key in required if key != 'user-invocable'):
            raise SystemExit('千问办公显示元数据不完整。')
        frontmatter = re.match(r'\A---\n(.*?)\n---\n', text, re.DOTALL)
        if not frontmatter:
            raise SystemExit('Skill frontmatter 无效。')
        header = re.sub(r'^description:.*\n?', '', frontmatter.group(1), flags=re.MULTILINE).rstrip()
        for key in required:
            header += '\n' + key + ': ' + json.dumps(localized[key], ensure_ascii=False)
        header += '\nagent_created: true'
        text = '---\n' + header + '\n---\n' + text[frontmatter.end():]
    return text.encode('utf-8')


def package(source, output):
    name = source.name
    entry = source / 'SKILL.md'
    if not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', name) or not entry.is_file():
        raise SystemExit('需要包含 SKILL.md 的标准 skill 文件夹。')
    text = entry.read_text(encoding='utf-8')
    if not re.search(r'^name:\s*' + re.escape(name) + r'\s*$', text, re.MULTILINE):
        raise SystemExit('Skill 名称与文件夹不一致。')
    files = resource_files(source)
    # Resolve inputs before opening the output, so a bad source keeps an existing package intact.
    contents = []
    for file in files:
        if file.is_symlink() or not file.is_file():
            raise SystemExit('不分发符号链接或缺失文件。')
        contents.append((file, entry_bytes(source) if file == entry else file.read_bytes()))
    validate_references(source, files)
    if len(contents) > MAX_FILES or sum(len(data) for _, data in contents) > MAX_PACKAGE_BYTES or any(
            len(data) > MAX_FILE_BYTES for _, data in contents):
        raise SystemExit('技能文件超过安装程序允许的数量或容量。')
    keys = [file.relative_to(source).as_posix() for file, _ in contents]
    if len({key.casefold() for key in keys}) != len(keys) or any(
            not re.fullmatch(r'[a-zA-Z0-9._/-]+', key) for key in keys):
        raise SystemExit('技能文件名不符合跨平台安装规则。')
    digests = {}
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = tempfile.NamedTemporaryFile(dir=output.parent, suffix='.zip', delete=False)
    temporary.close()
    staged = pathlib.Path(temporary.name)
    with zipfile.ZipFile(staged, 'w', zipfile.ZIP_DEFLATED) as archive:
        for file, data in contents:
            relative = f'{name}/{file.relative_to(source).as_posix()}'
            item = zipfile.ZipInfo(relative, PACKAGE_TIMESTAMP)
            item.create_system = 3
            # Portable Skills are small; stored entries also remove zlib-version differences.
            item.compress_type = zipfile.ZIP_STORED
            item.external_attr = PORTABLE_FILE_MODE << 16
            archive.writestr(item, data)
            digests[relative] = hashlib.sha256(data).hexdigest()
    os.replace(staged, output)
    manifest = {'skill': name, 'platform': 'universal', 'files': digests,
                'sha256': hashlib.sha256(output.read_bytes()).hexdigest()}
    output.with_suffix('.manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(f'已生成 {output}，包含 {len(files)} 个文件。')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=pathlib.Path,
                        default=pathlib.Path(__file__).resolve().parents[2] / 'skills' / 'dt-cli')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    package(args.source, args.output)


if __name__ == '__main__':
    main()
