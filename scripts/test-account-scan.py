"""Read-only SSH config transport, using temporary fixtures only."""
import base64
import contextlib
import importlib.util
import json
import sqlite3
import os
import pathlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('account_scan', pathlib.Path(__file__).with_name('remote_account_scan.py'))
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)


class NativeTransport(unittest.TestCase):
    def test_omp_overflow_numbers_skip_only_the_bad_pool_row(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'agent.db'
            db = sqlite3.connect(path)
            try:
                db.executescript((pathlib.Path(__file__).parent.parent / 'crates/core/tests/fixtures/omp-auth.sql').read_text())
                exact_expiry = 9223372036854775807
                db.execute("INSERT INTO auth_credentials VALUES(90,'google-gemini-cli','oauth',?,NULL,NULL,1,1)",
                           (json.dumps({'access': 'PRIVATE-EXACT-INTEGER', 'expires': exact_expiry, 'projectId': 'project-exact'}),))
                db.commit()
                expected = json.loads(scan.sqlite_document(path))
                bad_numbers = ['NaN', 'Infinity', '-Infinity', '1e400', '-1e400', '1.9e308',
                               '9' * 400, '-' + '9' * 400]
                for index, number in enumerate(bad_numbers, 100):
                    raw = '{"access":"PRIVATE-BAD-NUMBER","expires":' + number + '}'
                    db.execute("INSERT INTO auth_credentials VALUES(?,'google-gemini-cli','oauth',?,NULL,NULL,1,1)", (index, raw))
                db.commit()
                before = path.read_bytes()
                raw = scan.sqlite_document(path)
                actual = json.loads(raw)
                self.assertEqual(actual, expected, 'one unrepresentable number must not invalidate valid pool siblings')
                json.dumps(actual, allow_nan=False)
                self.assertIs(type(actual['google-gemini-cli#90']['expires']), int)
                self.assertEqual(actual['google-gemini-cli#90']['expires'], exact_expiry)
                self.assertNotIn(b'PRIVATE-BAD-NUMBER', raw)
                self.assertEqual(path.read_bytes(), before)
            finally:
                db.close()

    def test_omp_database_projects_only_supported_enabled_pool_and_watches_wal(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {'HOME': directory, 'USERPROFILE': directory}, clear=True):
            root = pathlib.Path(directory)
            path = root / 'agent.db'
            db = sqlite3.connect(path)
            try:
                db.executescript('PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;')
                db.executescript((pathlib.Path(__file__).parent.parent / 'crates/core/tests/fixtures/omp-auth.sql').read_text())
                db.execute("INSERT INTO auth_credentials VALUES(10,'google-gemini-cli','oauth',?,NULL,NULL,1,1)", ('{"access":"PRIVATE-MALFORMED","expires":NaN}',))
                db.commit()
                wal = root / 'agent.db-wal'
                before = path.read_bytes(), wal.read_bytes()
                value = json.loads(scan.sqlite_document(path))
                self.assertEqual(set(value), {'google-gemini-cli#1', 'google-gemini-cli#7', 'github-copilot#3', 'anthropic#8', 'openai-codex#9'})
                self.assertEqual(value['google-gemini-cli#1']['projectId'], 'project-one')
                self.assertNotIn('refresh', value['google-gemini-cli#1'])
                self.assertEqual(value['github-copilot#3']['refresh'], 'gho_PRIVATE-GITHUB')
                for excluded in ('PRIVATE-DO-NOT-PARSE', 'PRIVATE-GEMINI-DISABLED', 'PRIVATE-ANTIGRAVITY', 'PRIVATE-UNRELATED-FIELD', 'PRIVATE-METADATA'):
                    self.assertNotIn(excluded, json.dumps(value))
                self.assertEqual(before, (path.read_bytes(), wal.read_bytes()))
                request = dict(action='fingerprints', targets=[dict(provider='omp', roots=[directory], env='', patterns=['agent.db'], primary=directory)])
                initial = scan.run_request(request)
                db.execute("UPDATE auth_credentials SET disabled_cause='signed out' WHERE id=1")
                db.commit()
                with patch.object(scan, 'sqlite_document', side_effect=AssertionError('fingerprint polling must not read credentials')):
                    changed = scan.run_request(request)
                self.assertNotEqual(initial, changed)
                self.assertEqual(path.read_bytes(), before[0])
                self.assertNotIn('google-gemini-cli#1', json.loads(scan.sqlite_document(path)))
            finally:
                db.close()

    def test_omp_xdg_profile_data_root_finds_agent_database(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            data = home / 'data/omp/profiles/work'
            data.mkdir(parents=True)
            with contextlib.closing(sqlite3.connect(data / 'agent.db')) as db, db:
                db.executescript((pathlib.Path(__file__).parent.parent / 'crates/core/tests/fixtures/omp-auth.sql').read_text())
            request = dict(provider='omp', roots=[], env='PI_CODING_AGENT_DIR', patterns=['agent.db'], primary='~/.omp/agent')
            with patch.dict(os.environ, {'HOME': directory, 'USERPROFILE': directory, 'XDG_DATA_HOME': str(home / 'data'), 'OMP_PROFILE': 'work'}, clear=True), patch.object(scan.sys, 'platform', 'linux'):
                result = scan.collect(request)
            self.assertEqual(len(result['documents']), 1)
            self.assertEqual(result['documents'][0]['path'], str((data / 'agent.db').resolve()))
            self.assertFalse(result['errors'])

    def test_copilot_environment_precedes_files_with_only_public_projection(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            native = home / '.copilot'
            native.mkdir()
            (native / 'config.json').write_text('{"copilot_tokens":{"https://github.com:disk":"gho_DISK"}}')
            request = dict(provider='copilot', roots=[str(native)], env='', patterns=['config.json'], primary=str(native))
            for selected, tokens in (
                ('COPILOT_GITHUB_TOKEN', {'COPILOT_GITHUB_TOKEN': 'gho_PRIVATE_HIGH', 'GH_TOKEN': 'gho_PRIVATE_MIDDLE', 'GITHUB_TOKEN': 'gho_PRIVATE_LOW'}),
                ('GH_TOKEN', {'COPILOT_GITHUB_TOKEN': ' ', 'GH_TOKEN': 'gho_PRIVATE_MIDDLE', 'GITHUB_TOKEN': 'gho_PRIVATE_LOW'}),
                ('GITHUB_TOKEN', {'GITHUB_TOKEN': 'gho_PRIVATE_LOW'}),
            ):
                with self.subTest(selected=selected), patch.dict(os.environ, dict(HOME=str(home), USERPROFILE=str(home), **tokens), clear=True):
                    result = scan.collect(request)
                    self.assertEqual(result['documents'][0]['path'], 'env:' + selected)
                    value = json.loads(base64.b64decode(result['documents'][0]['data']))
                    self.assertEqual(value['aieyesTokenFingerprint'], scan.hashlib.sha256(tokens[selected].encode()).hexdigest())
                    self.assertEqual(set(value), {'token', 'aieyesTokenFingerprint'})
                    self.assertNotIn('PRIVATE', json.dumps(result) + json.dumps(value))
                    self.assertEqual(len([d for d in result['documents'] if d['path'].startswith('env:')]), 1)
                    # A reference to a lower-priority variable cannot borrow
                    # the currently selected token from a different variable.
                    other = 'env:GITHUB_TOKEN' if selected != 'GITHUB_TOKEN' else 'env:GH_TOKEN'
                    self.assertEqual(scan.collect(dict(request, pathsOnly=[other]))['documents'], [])
    def test_copilot_environment_fingerprint_watches_selection_without_reading_files(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            native = home / '.copilot'
            request = dict(provider='copilot', roots=[], env='', patterns=['config.json'], primary=str(native), fingerprint=True)
            with patch.dict(os.environ, {'HOME': str(home), 'USERPROFILE': str(home)}, clear=True), \
                    patch('builtins.open', side_effect=AssertionError('metadata only')), \
                    patch.object(pathlib.Path, 'open', side_effect=AssertionError('metadata only')):
                empty = scan.collect(request)
                os.environ['GH_TOKEN'] = 'gho_PRIVATE_GENERIC'
                self.assertEqual(scan.collect(request), empty, 'generic GitHub tokens need a Copilot directory')
                os.environ['COPILOT_GITHUB_TOKEN'] = 'gho_PRIVATE_ONE'
                first = scan.collect(request)
                self.assertNotEqual(first, empty)
                os.environ['GH_TOKEN'] = 'gho_PRIVATE_UNUSED'
                self.assertEqual(scan.collect(request), first)
                os.environ['COPILOT_GITHUB_TOKEN'] = 'gho_PRIVATE_TWO'
                self.assertNotEqual(scan.collect(request), first)
                self.assertNotIn('PRIVATE', json.dumps(scan.collect(request)))
    def test_metadata_poll_detects_new_glob_credentials_without_reading_them(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            request = dict(provider='codex', roots=[directory], env='', patterns=['accounts/*/auth.json'], fingerprint=True)
            from unittest.mock import patch
            with patch('builtins.open', side_effect=AssertionError('metadata polling must not read secrets')):
                before = scan.collect(request)
            auth = root / 'accounts/new/auth.json'
            auth.parent.mkdir(parents=True)
            auth.write_text('{"token":"PRIVATE"}')
            with patch('builtins.open', side_effect=AssertionError('metadata polling must not read secrets')):
                after = scan.collect(request)
                same = scan.collect(request)
            self.assertNotEqual(before['fingerprint'], after['fingerprint'])
            self.assertEqual(after, same)
            self.assertNotIn('documents', after)
            self.assertNotIn('PRIVATE', json.dumps(after))

    def test_metadata_batch_detects_sqlite_wal_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            path = root / 'state.vscdb'
            db = sqlite3.connect(path)
            try:
                db.executescript('PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE ItemTable(key text,value text); PRAGMA wal_checkpoint(TRUNCATE);')
                request = dict(action='fingerprints', targets=[dict(provider='cursor', roots=[], env='', patterns=[], primary=directory, pathsOnly=[str(path)])])
                before = scan.run_request(request)
                original = path.read_bytes()
                db.execute('INSERT INTO ItemTable VALUES(?,?)', ('cursorAuth/accessToken', 'PRIVATE'))
                db.commit()
                with patch.object(scan, 'sqlite_document', side_effect=AssertionError('metadata polling must not query the database')):
                    after = scan.run_request(request)
                self.assertNotEqual(before, after)
                self.assertEqual(path.read_bytes(), original)
                self.assertEqual(set(after['fingerprints']), {'cursor'})
                self.assertNotIn('PRIVATE', json.dumps(after))
            finally:
                db.close()

    def test_antigravity_ide_metadata_detects_new_stores_and_wal_without_reading_credentials(self):
        for platform, configured in [('darwin', False), ('win32', False), ('win32', True), ('linux', False), ('linux', True)]:
            with self.subTest(platform=platform, configured=configured), tempfile.TemporaryDirectory() as directory:
                home = pathlib.Path(directory)
                environment = {'HOME': directory, 'USERPROFILE': directory}
                if platform == 'darwin':
                    base = home / 'Library/Application Support'
                elif platform == 'win32':
                    base = home / ('custom-config' if configured else 'AppData/Roaming')
                    if configured:
                        environment['APPDATA'] = str(base)
                else:
                    base = home / ('custom-config' if configured else '.config')
                    if configured:
                        environment['XDG_CONFIG_HOME'] = str(base)
                target = dict(provider='antigravity', roots=['~/.gemini'], env='', patterns=['antigravity-oauth-token', 'settings.json'], primary='~/.gemini')
                request = dict(action='fingerprints', targets=[target])

                def poll(request=request):
                    with patch('builtins.open', side_effect=AssertionError('metadata polling must not read credentials')), \
                            patch.object(pathlib.Path, 'open', side_effect=AssertionError('metadata polling must not read credentials')), \
                            patch.object(scan, 'sqlite_document', side_effect=AssertionError('metadata polling must not project the database')), \
                            patch.object(scan.sqlite3, 'connect', side_effect=AssertionError('metadata polling must not query the database')):
                        result = scan.run_request(request)
                    self.assertEqual(set(result), {'fingerprints'})
                    self.assertEqual(set(result['fingerprints']), {'antigravity'})
                    self.assertEqual(set(result['fingerprints']['antigravity']), {'fingerprint'})
                    self.assertNotIn('PRIVATE', json.dumps(result))
                    return result

                with patch.dict(os.environ, environment, clear=True), patch.object(scan.sys, 'platform', platform):
                    before = poll()
                    explicit = dict(action='fingerprints', targets=[dict(target, pathsOnly=[])])
                    explicit_before = poll(explicit)
                    for name in ('Antigravity IDE', 'Antigravity'):
                        path = base / name / 'User/globalStorage/state.vscdb'
                        path.parent.mkdir(parents=True)
                        db = sqlite3.connect(path)
                        try:
                            db.executescript('PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE ItemTable(key text,value text); PRAGMA wal_checkpoint(TRUNCATE);')
                            created = poll()
                            self.assertNotEqual(before, created)
                            self.assertEqual(created, poll())
                            original = path.read_bytes()
                            db.execute('INSERT INTO ItemTable VALUES(?,?)', ('antigravityUnifiedStateSync.oauthToken', 'PRIVATE-IDE-CREDENTIAL'))
                            db.commit()
                            changed = poll()
                            self.assertNotEqual(created, changed)
                            self.assertEqual(changed, poll())
                            self.assertEqual(path.read_bytes(), original)
                        finally:
                            db.close()
                        before = poll()
                    self.assertEqual(explicit_before, poll(explicit))
                    with patch.object(scan, 'sqlite_document') as projection:
                        discovered = scan.collect(target)
                    projection.assert_not_called()
                    self.assertEqual(discovered['documents'], [])
                    self.assertEqual(discovered['roots'], [])

    def test_antigravity_ide_metadata_ignores_relative_config_roots(self):
        for platform, variable in [('win32', 'APPDATA'), ('linux', 'XDG_CONFIG_HOME')]:
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory:
                request = dict(provider='antigravity', roots=[], env='', patterns=[], primary=directory, fingerprint=True)
                with patch.dict(os.environ, {'HOME': directory, 'USERPROFILE': directory, variable: 'relative-config'}, clear=True), \
                        patch.object(scan.sys, 'platform', platform), patch.object(scan.os, 'stat', side_effect=FileNotFoundError) as metadata:
                    result = scan.collect(request)
                self.assertIn('fingerprint', result)
                self.assertTrue(all(pathlib.Path(call.args[0]).is_absolute() for call in metadata.call_args_list))
                self.assertFalse(any('globalStorage' in str(call.args[0]) for call in metadata.call_args_list))

    def test_kiro_database_is_projected_read_only_without_refresh_credentials(self):
        import base64, json, sqlite3
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            data = home / 'Library/Application Support' if scan.sys.platform == 'darwin' else home / '.local/share'
            path = data / 'kiro-cli/data.sqlite3'
            path.parent.mkdir(parents=True)
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                db.execute('CREATE TABLE auth_kv(key TEXT, value TEXT)')
                db.execute('CREATE TABLE state(key TEXT, value TEXT)')
                db.execute('INSERT INTO auth_kv VALUES(?, ?)', ('kirocli:odic:token', json.dumps(dict(access_token='ACCESS_FIXTURE', refresh_token='PRIVATE_REFRESH', expires_at='2100-01-01T00:00:00Z'))))
                db.execute('INSERT INTO auth_kv VALUES(?, ?)', ('kirocli:odic:device-registration', '{"client_secret":"PRIVATE_SECRET"}'))
                db.execute('INSERT INTO state VALUES(?, ?)', ('api.codewhisperer.profile', '{"arn":"profile"}'))
            before = path.read_bytes()
            reader = sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)
            with patch.dict(os.environ, {'HOME': str(home), 'USERPROFILE': str(home), 'XDG_DATA_HOME': str(data)}), \
                    patch.object(sqlite3, 'connect', return_value=reader):
                result = scan.collect(dict(provider='kiro', roots=[], env='', patterns=[], primary='~/.kiro'))
            with self.assertRaises(sqlite3.ProgrammingError):
                reader.execute('SELECT 1')
            self.assertFalse(result['errors'])
            self.assertEqual(len(result['documents']), 1)
            raw = base64.b64decode(result['documents'][0]['data']).decode()
            self.assertEqual(json.loads(raw)['kiro']['profileArn'], 'profile')
            self.assertIn('ACCESS_FIXTURE', raw)
            self.assertNotIn('PRIVATE', raw)
            self.assertEqual(path.read_bytes(), before)

    def test_remote_environment_and_reference_only_reads(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            root = home / 'custom'
            root.mkdir()
            (root / 'auth.json').write_text('{"token":"PRIVATE"}')
            (root / 'settings.json').write_text('{"key":"OTHER-PRIVATE"}')
            request = dict(provider='codex', roots=[], env='CODEX_HOME', patterns=['auth.json', 'settings.json'], primary='~/.codex')
            with patch.dict(os.environ, {'HOME': str(home), 'USERPROFILE': str(home), 'CODEX_HOME': str(root)}):
                result = scan.collect(request)
                self.assertEqual(len(result['documents']), 2)
                self.assertEqual(result['roots'], [str(root.resolve())])
                request.update(roots=[str(root)], env='', patterns=[], pathsOnly=[str(root / 'auth.json')])
                only = scan.collect(request)
                self.assertEqual([r['path'] for r in only['documents']], [str(root / 'auth.json')])
                self.assertEqual((root / 'auth.json').read_text(), '{"token":"PRIVATE"}')

    def test_relative_environment_and_large_or_invalid_inputs_are_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'auth.json').write_bytes(b'x' * (scan.LIMIT + 1))
            request = dict(provider='codex', roots=[directory], env='CODEX_HOME', patterns=['auth.json'])
            with patch.dict(os.environ, {'CODEX_HOME': 'relative-root'}):
                self.assertTrue(scan.collect(request)['errors'])
            request['env'] = ''
            result = scan.collect(request)
            self.assertFalse(result['documents'])
            self.assertTrue(result['errors'])


    def test_size_change_during_read_is_reported_per_path_like_rust(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'auth.json').write_text('{"token":"PRIVATE"}')
            real = os.stat

            def growing(target, *args, **kwargs):
                result = real(target, *args, **kwargs)
                if pathlib.Path(target).name == 'auth.json':
                    return os.stat_result((result.st_mode,) + tuple(result)[1:6] + (result.st_size + 7,) + tuple(result)[7:])
                return result
            request = dict(provider='codex', roots=[directory], env='', patterns=['auth.json'])
            with patch.object(scan.os, 'stat', growing):
                result = scan.collect(request)
            self.assertFalse(result['documents'])
            self.assertEqual(len(result['errors']), 1)
            self.assertTrue(result['errors'][0].endswith('auth.json：原生配置正在写入'))
            self.assertNotIn('PRIVATE', str(result))

    def test_workbuddy_database_is_projected_to_the_identity_and_never_shipped(self):
        import sqlite3
        fixture = pathlib.Path(__file__).resolve().parents[1] / 'crates/core/tests/fixtures/workbuddy/workbuddy.sql'
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with contextlib.closing(sqlite3.connect(root / 'workbuddy.db')) as db, db:
                db.executescript(fixture.read_text())
            request = dict(provider='workbuddy', roots=[directory], env='', patterns=['storage/skeleton/account-snapshot.json', 'workbuddy.db'])
            result = scan.collect(request)
            self.assertEqual(result['errors'], [])
            [document] = result['documents']
            projected = json.loads(base64.b64decode(document['data']))
            # Latest visible session: deleted rows never name the login.
            self.assertEqual(projected, {'workbuddy': {'userId': 'fixture-user'}})
            self.assertNotIn('PRIVATE', json.dumps(result))
            skeleton = root / 'storage/skeleton'
            skeleton.mkdir(parents=True)
            self.assertEqual(json.loads(base64.b64decode(scan.collect(request)['documents'][0]['data'])), {})
            (skeleton / 'account-snapshot.json').write_text('{"primary":{"version":1,"uid":"fixture-user","type":"personal"}}')
            paths = sorted(d['path'] for d in scan.collect(request)['documents'])
            self.assertEqual([pathlib.Path(p).name for p in paths], ['account-snapshot.json', 'workbuddy.db'])

    def test_database_projection_reads_wal_and_omits_unrelated_private_state(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            root = home / ('账户#%' if os.name == 'nt' else '账户?#%') / 'opencode'
            root.mkdir(parents=True)
            db = sqlite3.connect(root / 'opencode.db')
            db.executescript("PRAGMA journal_mode=WAL;CREATE TABLE credential(id text,integration_id text,value text,active integer,time_created integer);CREATE TABLE messages(body text);INSERT INTO messages VALUES('PRIVATE-CHAT');")
            for row in [('old','anthropic','{"type":"key","key":"OLD"}',0,9), ('live','anthropic','{"type":"oauth","access":"FIXTURE"}',1,1), ('new','github-copilot','{"type":"oauth","access":"gho_FIXTURE"}',None,3)]:
                db.execute('INSERT INTO credential VALUES(?,?,?,?,?)', row)
            db.commit()
            before = (root / 'opencode.db').read_bytes()
            with patch.dict(os.environ, {'HOME': str(home), 'USERPROFILE': str(home)}):
                request = dict(provider='opencode', roots=[str(root)], env='', patterns=['auth.json'], primary=str(root))
                result = scan.collect(request)
                self.assertEqual(result['errors'], [])
                self.assertEqual(len(result['documents']), 1)
                raw = base64.b64decode(result['documents'][0]['data'])
                self.assertNotIn(b'PRIVATE-CHAT', raw)
                self.assertEqual(json.loads(raw)['anthropic']['access'], 'FIXTURE')
                self.assertEqual((root / 'opencode.db').read_bytes(), before)
            db.close()

    def test_cursor_sqlite_plaintext_and_sealed_tokens_match_local_projection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            path = root / 'state.vscdb'
            db = sqlite3.connect(path)
            db.execute('CREATE TABLE ItemTable(key text, value blob)')
            db.executemany('INSERT INTO ItemTable VALUES(?,?)', [('cursorAuth/accessToken','cursor-safe-storage:v1:PRIVATE'), ('cursorAuth/cachedEmail','fixture@example.test'), ('cursorAuth/refreshToken','PRIVATE-REFRESH')])
            db.commit()
            request = dict(provider='cursor', roots=[str(root)], env='', patterns=[], primary=str(root), pathsOnly=[str(path)])
            result = scan.collect(request)
            raw = base64.b64decode(result['documents'][0]['data'])
            self.assertEqual(json.loads(raw), {'cursorAuth':{'encrypted':True, 'cachedEmail':'fixture@example.test'}})
            self.assertNotIn(b'PRIVATE', raw)
            db.close()

    def test_kimi_work_uses_relocated_desktop_storage_with_same_remote_transport(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            config = home / 'Library/Application Support/Kimi'
            config.mkdir(parents=True)
            share = home / '移动数据'
            (config / 'daimon-storage.json').write_text(json.dumps({'shareDir': str(share)}))
            native = share / 'daimon/config.json'
            native.parent.mkdir(parents=True)
            native.write_text('{"credentials":{"kimiWeb":{"userId":"fixture"}}}')
            kernel = share / 'daimon/runtime/kimi-code/home'
            kernel.mkdir(parents=True)
            with patch.dict(os.environ, {'HOME':str(home), 'USERPROFILE':str(home), 'KIMI_SHARE_DIR':''}), patch.object(scan.sys, 'platform', 'darwin'):
                result = scan.collect(dict(provider='kimi', roots=[], env='', patterns=['config.toml'], primary='~/.kimi-code'))
            self.assertEqual(result['errors'], [])
            self.assertIn(str(kernel.resolve()), result['roots'])
            self.assertEqual(pathlib.Path(result['documents'][0]['path']).resolve(), native.resolve())
            self.assertEqual(pathlib.Path(result['documents'][0]['root']).resolve(), kernel.resolve())

    def test_codebuddy_ide_identity_comes_from_the_history_owner_without_reading_messages(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            history = home / 'Library/Application Support/CodeBuddyExtension/Data/u1/CodeBuddy/u1/history'
            history.mkdir(parents=True)
            (history / 'PRIVATE').write_text('PRIVATE-CONTENT')
            with patch.dict(os.environ, {'HOME':str(home), 'USERPROFILE':str(home)}), patch.object(scan.sys, 'platform', 'darwin'):
                result = scan.collect(dict(provider='codebuddy', roots=[], env='', patterns=[], primary='~/.codebuddy'))
            self.assertEqual(result['errors'], [])
            self.assertEqual(len(result['documents']), 1)
            self.assertEqual(json.loads(base64.b64decode(result['documents'][0]['data'])), {'codebuddyHistoryUserId':'u1'})
            self.assertIn(str(history.resolve()), result['roots'])
            self.assertNotIn('PRIVATE', json.dumps(result))


if __name__ == '__main__':
    unittest.main()
