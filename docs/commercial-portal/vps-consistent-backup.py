#!/usr/bin/env python3
"""Run ONLY after explicit VPS backup authorization. Python standard library only.

Creates a private SQLite online-backup snapshot, then restores its bytes into a
separate private file and validates both. Does not start services or prove
application-level recovery, and does not replace an independent off-host backup.
Never copy a live database, change source PRAGMAs, or use immutable=1 with WAL.
Accept a backup only when CLI exit status is 0 AND metadata status is valid.
Failed runs remain private; do not treat .part files as backups. A failure during
the final directory fsync may leave valid metadata despite a nonzero CLI exit.
Use a trusted root-owned destination and no concurrent filesystem administrators.
"""

import argparse
from contextlib import closing
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import time
import uuid


class BackupError(Exception):
    """Safe, fixed diagnostic without database rows or SQLite error payloads."""


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def checked_path(path):
    path = Path(os.path.abspath(path))
    for component in (path, *path.parents):
        if component.is_symlink():
            raise BackupError('Symlink paths are prohibited.')
    return path.resolve()


def private_file(path):
    return os.fdopen(os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600), 'wb')


def readonly(path, timeout=0.1):
    return sqlite3.connect(Path(path).as_uri() + '?mode=ro', uri=True, timeout=timeout)


def check_ownership(path, destination=False, trusted_uids=None):
    if hasattr(os, 'geteuid'):
        euid = os.geteuid()
        if destination:
            allowed = {euid}
        else:
            allowed = {euid, 10001} if trusted_uids is None else set(trusted_uids)
        s = path.stat()
        if s.st_uid not in allowed:
            raise BackupError('Untrusted owner.')
        if destination:
            if s.st_mode & 0o077:
                raise BackupError('Existing destination root must be private (0700).')
        else:
            if (s.st_mode & 0o022) and not (s.st_mode & 0o1000):
                raise BackupError('Writable ancestor without sticky bit.')


def sync_directory(path):
    if os.name == 'posix':
        descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)


REQUIRED_COLUMNS = {
    'users': {'id', 'email', 'password_hash'},
    'devices': {'id', 'user_id', 'token_hash', 'revoked'},
    'sessions': {'token_hash', 'user_id', 'expires_at'},
    'grants': {'id', 'user_id', 'access_hash', 'refresh_hash', 'revoked', 'expires_at'},
}


def validate_snapshot(path, expected_sha, deadline=None, now=None):
    """Read snapshot only; return aggregate counts, never records or identifiers."""
    if sha256(path) != expected_sha:
        raise BackupError('Snapshot checksum mismatch.')
    now = int(time.time()) if now is None else now
    with closing(readonly(path)) as con:
        if deadline is not None:
            con.set_progress_handler(lambda: int(time.monotonic() >= deadline), 1000)
        if con.execute('PRAGMA integrity_check').fetchall() != [('ok',)]:
            raise BackupError('Snapshot integrity check failed.')
        if con.execute('PRAGMA foreign_key_check').fetchone() is not None:
            raise BackupError('Snapshot foreign key check failed.')
        counts = {}
        for table, req_cols in REQUIRED_COLUMNS.items():
            exists = con.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)).fetchone()
            if not exists:
                raise BackupError(f'Required table {table} missing.')
            columns = {row[1] for row in con.execute('PRAGMA table_info("' + table + '")')}
            if not req_cols.issubset(columns):
                raise BackupError(f'Required column missing in table {table}.')
            counts[table] = {'total': con.execute('SELECT COUNT(*) FROM "' + table + '"').fetchone()[0]}
            conditions, parameters = [], []
            if 'revoked' in columns:
                conditions.append('revoked = 0')
            if 'expires_at' in columns:
                conditions.append('expires_at > ?')
                parameters.append(now)
            if conditions:
                counts[table]['active'] = con.execute('SELECT COUNT(*) FROM "' + table + '" WHERE ' + ' AND '.join(conditions), parameters).fetchone()[0]
    return counts


def create_backup(source, destination_root, timeout=60, project_root=None, service_uid=10001):
    if project_root is None:
        raise BackupError('An explicit deployed project root is required.')
    if not math.isfinite(timeout) or timeout <= 0 or timeout > 3600:
        raise BackupError('Timeout must be between 0 and 3600 seconds.')
    source = checked_path(source)
    destination_root = checked_path(destination_root)
    project_root = checked_path(project_root)
    for forbidden in (source.parent, project_root, Path('/var/lib/docker').resolve()):
        if destination_root == forbidden or forbidden in destination_root.parents:
            raise BackupError('Destination must be outside source, project and Docker directories.')
    if not source.is_file():
        raise BackupError('Source database is not a regular file.')
    if hasattr(os, 'geteuid') and os.geteuid() == 0:
        trusted_source = {0, service_uid}
        check_ownership(source, trusted_uids=trusted_source)
        for suffix in ('-wal', '-shm'):
            sidecar = Path(str(source) + suffix)
            if sidecar.exists():
                check_ownership(sidecar, trusted_uids=trusted_source)
        for parent in source.parents:
            if parent.exists():
                parent_trusted = trusted_source if parent == source.parent else {0}
                check_ownership(parent, trusted_uids=parent_trusted)
    new_dirs = []
    if destination_root.exists():
        if not destination_root.is_dir():
            raise BackupError('Destination root must be a directory.')
        if os.name == 'posix' and (destination_root.stat().st_mode & 0o077):
            raise BackupError('Existing destination root must be private (0700).')
        if hasattr(os, 'geteuid') and os.geteuid() == 0:
            check_ownership(destination_root, destination=True)
            for parent in destination_root.parents:
                if parent.exists():
                    check_ownership(parent, destination=False, trusted_uids={0})
        new_dirs.append(destination_root)
    else:
        curr = destination_root
        to_create = []
        while not curr.exists():
            to_create.append(curr)
            curr = curr.parent
        if hasattr(os, 'geteuid') and os.geteuid() == 0:
            check_ownership(curr, destination=False, trusted_uids={0})
            for parent in curr.parents:
                if parent.exists():
                    check_ownership(parent, destination=False, trusted_uids={0})
        new_dirs.append(curr)
        for d in reversed(to_create):
            d.mkdir(mode=0o700, exist_ok=False)
            new_dirs.append(d)
    run = destination_root / (datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ') + '-' + uuid.uuid4().hex)
    run.mkdir(mode=0o700, exist_ok=False)
    new_dirs.append(run)
    metadata = {'status': 'invalid', 'directory': str(run), 'created_at_utc': datetime.now(timezone.utc).isoformat(),
                'scope': 'database-only; no services started; not off-host recovery proof'}
    with private_file(run / 'metadata.json') as stream:
        stream.write((json.dumps(metadata, indent=2) + '\n').encode())
    snapshot = run / 'snapshot.db.part'
    restored = run / 'restore-check.db.part'
    deadline = time.monotonic() + timeout
    now = int(time.time())

    def progress(status, remaining, total):
        if time.monotonic() >= deadline:
            raise BackupError('Backup timeout exceeded; artifacts are invalid.')

    try:
        with private_file(snapshot):
            pass
        with closing(readonly(source)) as live, closing(sqlite3.connect(snapshot, timeout=0.1)) as target:
            live.backup(target, pages=128, progress=progress, sleep=0.05)
            # Change ONLY the private snapshot to standalone rollback journal mode.
            target.execute('PRAGMA journal_mode=DELETE')
        digest = sha256(snapshot)
        counts = validate_snapshot(snapshot, digest, deadline, now)
        with snapshot.open('rb') as inp, private_file(restored) as out:
            shutil.copyfileobj(inp, out, 1024 * 1024)
            out.flush()
            os.fsync(out.fileno())
        restored_counts = validate_snapshot(restored, digest, deadline, now)
        if restored_counts != counts:
            raise BackupError('Restored aggregate counts do not match.')
        progress(0, 0, 0)
        with snapshot.open('r+b') as stream:
            os.fsync(stream.fileno())
        snapshot.rename(run / 'snapshot.db')
        restored.rename(run / 'restore-check.db')
        metadata.update(status='valid', sha256=digest, counts=counts, counts_at_epoch=now,
                        bytes=(run / 'snapshot.db').stat().st_size,
                        integrity_check='ok', foreign_key_check='ok', restore_checksum='match', restore_counts='match',
                        active_definition='revoked = 0 when present AND expires_at > counts_at_epoch when present')
        with private_file(run / 'metadata.json.part') as stream:
            stream.write((json.dumps(metadata, indent=2) + '\n').encode())
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(run / 'metadata.json.part', run / 'metadata.json')
        for d in new_dirs:
            sync_directory(d)
        return metadata
    except sqlite3.Error:
        raise BackupError('SQLite backup or validation failed; artifacts are invalid.') from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path('/var/lib/docker/volumes/deployment_service-data/_data/service.db'))
    parser.add_argument('--destination-root', type=Path, default=Path('/root/remotecontroller-backups'))
    parser.add_argument('--project-root', type=Path, required=True, help='Actual deployed checkout root; prohibited as destination')
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--service-uid', type=int, default=10001, help='UID of the service user inside container')
    args = parser.parse_args()
    try:
        result = create_backup(args.source, args.destination_root, args.timeout, args.project_root, args.service_uid)
    except BackupError as error:
        print(str(error), file=sys.stderr)
        return 1
    except OSError:
        print('Filesystem operation failed; created artifacts remain private and MUST NOT be accepted as a valid backup.', file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main())
