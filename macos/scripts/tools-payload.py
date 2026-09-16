#!/usr/bin/env python3
"""Build-time inventory and validation for the single Tools resource format."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

LAUNCHER = '''#!/bin/sh
set -eu
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
unset NODE_OPTIONS NODE_PATH
exec "$ROOT/node/bin/node" "$ROOT/cli/node_modules/astroshot/bin/astroshot.mjs" "$@"
'''

LOAD_COMMANDS = (
    'LC_LOAD_DYLIB',
    'LC_LOAD_WEAK_DYLIB',
    'LC_REEXPORT_DYLIB',
    'LC_LOAD_UPWARD_DYLIB',
    'LC_LAZY_LOAD_DYLIB',
)

SYSTEM_PREFIXES = ('/usr/lib/', '/System/Library/')


def node_arch(root):
    archs = subprocess.check_output(
        ['/usr/bin/lipo', '-archs', str(root / 'node/bin/node')], text=True
    ).split()
    if archs == ['arm64']:
        return 'arm64', {'arm64'}
    if archs == ['x86_64']:
        return 'x64', {'x86_64'}
    raise ValueError(f'unsupported node architecture: {" ".join(archs)}')


def resolve_load(binary, dep, root, executable):
    if any(dep.startswith(prefix) for prefix in SYSTEM_PREFIXES):
        resolved = Path(dep).resolve()
        if not str(resolved).startswith(SYSTEM_PREFIXES):
            raise ValueError(f'escaping dylib: {binary}: {dep}')
        return
    if dep.startswith('@rpath'):
        raise ValueError(f'rpath dylib: {binary}: {dep}')
    if dep.startswith('@loader_path'):
        rest = dep[len('@loader_path'):].lstrip('/')
        resolved = (binary.parent / rest).resolve() if rest else binary.parent.resolve()
    elif dep.startswith('@executable_path'):
        rest = dep[len('@executable_path'):].lstrip('/')
        resolved = (executable.parent / rest).resolve() if rest else executable.parent.resolve()
    else:
        raise ValueError(f'non-system dylib: {binary}: {dep}')
    try:
        resolved.relative_to(root.resolve())
    except ValueError as error:
        raise ValueError(f'escaping dylib: {binary}: {dep}') from error
    if not resolved.is_file():
        raise ValueError(f'missing dylib: {binary}: {dep}')


def inventory(root, allowed_archs, executable):
    result = {}
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            p = Path(directory) / name
            if p.is_symlink():
                raise ValueError(f"symlink: {p}")
            if not p.is_dir() and not p.is_file():
                raise ValueError(f"special file: {p}")
        for name in files:
            p = Path(directory) / name
            rel = p.relative_to(root).as_posix()
            if rel == 'manifest.json':
                continue
            result[rel] = {'sha256': hashlib.sha256(p.read_bytes()).hexdigest(),
                           'executable': bool(p.stat().st_mode & 0o111)}
            if 'Mach-O' not in subprocess.check_output(['/usr/bin/file', '-b', str(p)], text=True):
                continue
            archs = subprocess.check_output(['/usr/bin/lipo', '-archs', str(p)], text=True).split()
            for arch in archs:
                if arch not in allowed_archs:
                    raise ValueError(f'wrong architecture: {p}: {arch}')
            command = ''
            for line in subprocess.check_output(['/usr/bin/otool', '-l', str(p)], text=True).splitlines():
                line = line.strip()
                if line.startswith('cmd '):
                    command = line.split()[1]
                    continue
                if command == 'LC_RPATH' and line.startswith('path '):
                    dep = line.split(' ', 1)[1].split(' (offset')[0]
                    raise ValueError(f'rpath: {p}: {dep}')
                if command == 'LC_DYLD_ENVIRONMENT' and line.startswith('name '):
                    raise ValueError(f'dyld environment: {p}: {line}')
                if command in LOAD_COMMANDS and line.startswith('name '):
                    dep = line.split(' ', 1)[1].split(' (offset')[0]
                    resolve_load(p, dep, root, executable)
    return result


def prune_foreign_arch(root):
    node = root / 'node/bin/node'
    if not node.is_file():
        raise ValueError('missing file: node/bin/node')
    _, allowed_archs = node_arch(root)
    for directory, _, files in os.walk(root, followlinks=False):
        for name in files:
            p = Path(directory) / name
            if 'Mach-O' not in subprocess.check_output(['/usr/bin/file', '-b', str(p)], text=True):
                continue
            archs = subprocess.check_output(['/usr/bin/lipo', '-archs', str(p)], text=True).split()
            if any(arch not in allowed_archs for arch in archs):
                keep = [arch for arch in archs if arch in allowed_archs]
                if not keep:
                    p.unlink()
                    continue
                subprocess.check_call(['/usr/bin/lipo', str(p), '-thin', keep[0], '-output', str(p)])


def validate(root, write=False):
    if root.is_symlink():
        raise ValueError('symlink payload root')
    node = root / 'node/bin/node'
    if not node.is_file() or node.is_symlink():
        raise ValueError('missing file: node/bin/node')
    arch, allowed_archs = node_arch(root)
    entries = inventory(root, allowed_archs, node)
    required = ['node/bin/node', 'bin/astroshot',
                'cli/node_modules/astroshot/bin/astroshot.mjs',
                'cli/node_modules/astroshot/node_modules/@archastro/astroshot/bin/astroshot.mjs']
    for name in required:
        if name not in entries:
            raise ValueError(f'missing file: {name}')
    if not any(p.startswith('skills/') and p.endswith('/SKILL.md') for p in entries):
        raise ValueError('missing skills')
    for name in ['node/bin/node', 'bin/astroshot']:
        if not entries[name]['executable']:
            raise ValueError(f'not executable: {name}')
    if (root / 'bin/astroshot').read_text() != LAUNCHER:
        raise ValueError('network or noncanonical launcher')
    manifest = {'arch': arch, 'files': entries, 'format': 1}
    if write:
        (root / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    elif json.loads((root / 'manifest.json').read_text()) != manifest:
        raise ValueError('integrity manifest mismatch')


if __name__ == '__main__':
    try:
        if sys.argv[1] == 'launcher':
            Path(sys.argv[2]).write_text(LAUNCHER)
        elif sys.argv[1] == 'prune':
            prune_foreign_arch(Path(sys.argv[2]))
        else:
            validate(Path(sys.argv[2]), write=sys.argv[1] == 'seal')
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f'tools payload: {error}')
