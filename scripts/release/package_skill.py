#!/usr/bin/env python3
"""Export a portable skill without installing it into any Agent's directories."""
import argparse
import hashlib
import json
import pathlib
import re
import zipfile


def entry_bytes(source, platform):
    text = (source / 'SKILL.md').read_text(encoding='utf-8')
    if platform == 'qwenwork':
        localized = json.loads((source / 'agents' / 'qwenwork.json').read_text(encoding='utf-8'))
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
        text = '---\n' + header + '\n---\n' + text[frontmatter.end():]
    elif platform == 'workbuddy':
        text = text.replace('\n---\n', '\nagent_created: true\n---\n', 1)
    return text.encode('utf-8')


def package(source, output, platform='standard'):
    name = source.name
    entry = source / 'SKILL.md'
    if not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', name) or not entry.is_file():
        raise SystemExit('需要包含 SKILL.md 的标准 skill 文件夹。')
    text = entry.read_text(encoding='utf-8')
    if not re.search(r'^name:\s*' + re.escape(name) + r'\s*$', text, re.MULTILINE):
        raise SystemExit('Skill 名称与文件夹不一致。')
    files = [entry, source / 'LICENSE', *sorted((source / 'references').glob('*.md'))]
    if platform == 'standard':
        metadata = source / 'agents' / 'openai.yaml'
        if metadata.exists():
            files.append(metadata)
    examples = source / '.skill-metadata.yaml'
    if examples.exists():
        files.append(examples)
    elif platform == 'qwenwork':
        raise SystemExit('千问办公包缺少推荐任务元数据。')
    # Resolve inputs before opening the output, so a bad source keeps an existing package intact.
    contents = []
    for file in files:
        if file.is_symlink() or not file.is_file():
            raise SystemExit('不分发符号链接或缺失文件。')
        contents.append((file, entry_bytes(source, platform) if file == entry else file.read_bytes()))
    digests = {}
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as archive:
        for file, data in contents:
            relative = f'{name}/{file.relative_to(source).as_posix()}'
            item = zipfile.ZipInfo(relative, (2026, 1, 1, 0, 0, 0))
            item.compress_type = zipfile.ZIP_DEFLATED
            item.external_attr = 0o100644 << 16
            archive.writestr(item, data)
            digests[relative] = hashlib.sha256(data).hexdigest()
    manifest = {'skill': name, 'platform': platform, 'files': digests,
                'sha256': hashlib.sha256(output.read_bytes()).hexdigest()}
    output.with_suffix('.manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(f'已生成 {output}，包含 {len(files)} 个文件。')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=pathlib.Path,
                        default=pathlib.Path(__file__).resolve().parents[2] / 'skills' / 'dt-cli')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--platform', choices=('standard', 'qwenwork', 'workbuddy'), default='standard')
    args = parser.parse_args()
    package(args.source, args.output, args.platform)


if __name__ == '__main__':
    main()
