import importlib.util
from contextlib import closing, redirect_stdout, redirect_stderr
import io
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch, Mock

MODULE = Path(__file__).resolve().parents[1] / 'vps-consistent-backup.py'


class BackupTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(MODULE.exists(), 'backup implementation is missing')
        spec = importlib.util.spec_from_file_location('backup_tool', MODULE)
        self.tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.tool)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.live = self.root / 'live'
        self.live.mkdir()
        self.db = self.live / 'service.db'
        self.writer = sqlite3.connect(self.db)
        self.addCleanup(self.writer.close)
        self.writer.executescript('''PRAGMA journal_mode=WAL;
            PRAGMA wal_autocheckpoint=0;
            CREATE TABLE users(id TEXT PRIMARY KEY, email TEXT, password_hash TEXT);
            CREATE TABLE devices(id TEXT, user_id TEXT, token_hash TEXT, revoked INTEGER);
            CREATE TABLE sessions(token_hash TEXT, user_id TEXT REFERENCES users(id), expires_at INTEGER);
            CREATE TABLE grants(id TEXT, user_id TEXT, access_hash TEXT, refresh_hash TEXT, revoked INTEGER, expires_at INTEGER);
            INSERT INTO users VALUES ('private-id', 'secret@example.test', 'secret-hash');
            INSERT INTO devices VALUES ('private-device', 'private-id', 'secret-device-token', 0);
            INSERT INTO sessions VALUES ('secret-session-token', 'private-id', 4102444800);
            INSERT INTO grants VALUES ('private-grant', 'private-id', 'secret-access', 'secret-refresh', 0, 4102444800);''')
        self.writer.commit()

    def run_backup(self, destination=None, **kwargs):
        kwargs.setdefault('project_root', self.root / 'project')
        return self.tool.create_backup(self.db, destination or self.root / 'backups', **kwargs)

    def test_cli_help_works_with_script_copied_to_short_path(self):
        with patch.object(self.tool, '__file__', '/vps-consistent-backup.py'), patch('sys.argv', ['backup', '--help']), redirect_stdout(io.StringIO()):
            with self.assertRaises(SystemExit) as exit_result:
                self.tool.main()
        self.assertEqual(exit_result.exception.code, 0)

    def test_cli_requires_explicit_project_root(self):
        with patch('sys.argv', ['backup']), redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as exit_result:
                self.tool.main()
        self.assertEqual(exit_result.exception.code, 2)

    def test_api_requires_explicit_project_root(self):
        with self.assertRaises(self.tool.BackupError):
            self.tool.create_backup(self.db, self.root / 'backups')
        self.assertFalse((self.root / 'backups').exists())

    def test_committed_uncheckpointed_wal_restores_and_source_unchanged(self):
        before = {p: p.read_bytes() for p in (self.db, Path(str(self.db) + '-wal'))}
        result = self.run_backup()
        self.assertEqual(result['status'], 'valid')
        self.assertEqual(result['counts']['users']['total'], 1)
        self.assertEqual(result['counts']['sessions']['active'], 1)
        restored = Path(result['directory']) / 'restore-check.db'
        with closing(sqlite3.connect(restored)) as con:
            self.assertEqual(con.execute('SELECT COUNT(*) FROM users').fetchone()[0], 1)
        for path, content in before.items():
            self.assertEqual(path.read_bytes(), content)
        self.assertFalse(Path(str(restored) + '-wal').exists())
        self.assertEqual(self.tool.sha256(restored), result['sha256'])
        if os.name == 'posix':
            self.assertEqual(Path(result['directory']).stat().st_mode & 0o777, 0o700)
            for path in Path(result['directory']).iterdir():
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_metadata_contains_only_approved_aggregates(self):
        result = self.run_backup()
        metadata = (Path(result['directory']) / 'metadata.json').read_text()
        for secret in ('private-id', 'secret@example.test', 'private-device', 'email', 'token'):
            self.assertNotIn(secret, metadata)
        self.assertEqual(set(result['counts']), {'users', 'devices', 'sessions', 'grants'})
        self.assertEqual(json.loads(metadata)['status'], 'valid')

    def test_destination_under_live_or_project_rejected(self):
        for destination in (self.live / 'backups', self.root / 'project' / 'backups'):
            with self.subTest(destination=destination):
                with self.assertRaises(self.tool.BackupError):
                    self.run_backup(destination, project_root=self.root / 'project')
                self.assertFalse(destination.exists())

    def test_unique_runs_do_not_overwrite(self):
        first = self.run_backup()
        second = self.run_backup()
        self.assertNotEqual(first['directory'], second['directory'])
        self.assertTrue((Path(first['directory']) / 'snapshot.db').exists())

    def test_checksum_failure_and_foreign_key_violation_rejected(self):
        result = self.run_backup()
        with self.assertRaises(self.tool.BackupError):
            self.tool.validate_snapshot(Path(result['directory']) / 'snapshot.db', '0' * 64)
        self.writer.execute("INSERT INTO sessions VALUES ('secret-invalid', 'missing', 4102444800)")
        self.writer.commit()
        with self.assertRaises(self.tool.BackupError):
            self.run_backup()
        for metadata in (self.root / 'backups').glob('*/metadata.json'):
            if json.loads(metadata.read_text())['status'] != 'valid':
                self.assertFalse((metadata.parent / 'snapshot.db').exists())

    @unittest.skipUnless(os.name == 'posix', 'POSIX symlink privileges required')
    def test_symlink_source_and_destination_rejected(self):
        link = self.root / 'link.db'
        link.symlink_to(self.db)
        with self.assertRaises(self.tool.BackupError):
            self.tool.create_backup(link, self.root / 'backups', project_root=self.root / 'project')
        target = self.root / 'target'
        target.mkdir()
        linkdir = self.root / 'linked'
        linkdir.symlink_to(target, target_is_directory=True)
        with self.assertRaises(self.tool.BackupError):
            self.run_backup(linkdir / 'backups')

    def test_wal_writer_pending_rows_not_in_snapshot(self):
        self.writer.execute('BEGIN EXCLUSIVE')
        self.writer.execute("INSERT INTO users VALUES ('pending', 'hidden@example.test', 'hidden-hash')")
        # WAL readers should still see the committed snapshot, without pending rows.
        result = self.run_backup(timeout=2)
        self.assertEqual(result['counts']['users']['total'], 1)
        self.writer.rollback()
        with self.assertRaises(self.tool.BackupError):
            self.run_backup(timeout=0)

    def test_exclusive_rollback_lock_times_out_without_valid_snapshot(self):
        self.writer.close()
        self.writer = sqlite3.connect(self.db)
        self.addCleanup(self.writer.close)
        self.writer.execute('PRAGMA journal_mode=DELETE')
        self.writer.execute('BEGIN EXCLUSIVE')
        started = time.monotonic()
        with self.assertRaises(self.tool.BackupError):
            self.run_backup(timeout=0.2)
        self.assertLess(time.monotonic() - started, 2)
        self.writer.rollback()
        metadata = next((self.root / 'backups').glob('*/metadata.json'))
        self.assertEqual(json.loads(metadata.read_text())['status'], 'invalid')
        self.assertFalse((metadata.parent / 'snapshot.db').exists())

    def test_wrong_empty_database_is_rejected(self):
        empty = self.live / 'wrong.db'
        with closing(sqlite3.connect(empty)):
            pass
        with self.assertRaises(self.tool.BackupError):
            self.tool.create_backup(empty, self.root / 'backups', project_root=self.root / 'project')
        metadata = next((self.root / 'backups').glob('*/metadata.json'))
        self.assertEqual(json.loads(metadata.read_text())['status'], 'invalid')

    def test_missing_critical_column_is_rejected(self):
        self.writer.execute('ALTER TABLE users DROP COLUMN password_hash')
        self.writer.commit()
        with self.assertRaises(self.tool.BackupError):
            self.run_backup()

    def test_untrusted_owner_and_writable_ancestor_rejected_with_mocked_posix_stat(self):
        fake_path = Mock()
        fake_path.stat.return_value = SimpleNamespace(st_uid=1234, st_mode=0o40700)
        with patch.object(self.tool.os, 'geteuid', return_value=0, create=True):
            with self.assertRaises(self.tool.BackupError):
                self.tool.check_ownership(fake_path, destination=True)
            with self.assertRaises(self.tool.BackupError):
                self.tool.check_ownership(fake_path)
            fake_path.stat.return_value = SimpleNamespace(st_uid=0, st_mode=0o40777)
            with self.assertRaises(self.tool.BackupError):
                self.tool.check_ownership(fake_path)
            fake_path.stat.return_value = SimpleNamespace(st_uid=0, st_mode=0o41777)
            self.tool.check_ownership(fake_path)
            # Service UID 10001 is allowed for source, but prohibited for destination
            fake_path.stat.return_value = SimpleNamespace(st_uid=10001, st_mode=0o100644)
            self.tool.check_ownership(fake_path)
            with self.assertRaises(self.tool.BackupError):
                self.tool.check_ownership(fake_path, destination=True)

    @unittest.skipUnless(os.name == 'posix', 'POSIX directory fsync requires Linux')
    def test_directory_entries_are_synced_before_success(self):
        actual = self.tool.sync_directory
        calls = []
        def record(path):
            calls.append(path)
            actual(path)
        with patch.object(self.tool, 'sync_directory', side_effect=record):
            result = self.run_backup(self.root / 'new' / 'backups')
        for path in (self.root, self.root / 'new', self.root / 'new' / 'backups', Path(result['directory'])):
            self.assertIn(path, calls)


if __name__ == '__main__':
    unittest.main()
