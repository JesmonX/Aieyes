#!/usr/bin/env python3
"""Offline coordinator fault tests. No real accounts or processes are touched."""
import base64
import builtins
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('auth_helper', pathlib.Path(__file__).with_name('remote_codex_auth.py'))
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)
scan_switch_processes = a.switch_running


def credential(user, refresh='initial'):
    claims = {'https://api.openai.com/auth': {'chatgpt_user_id': user, 'chatgpt_account_id': 'team', 'chatgpt_plan_type': 'plus'}}
    token = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=')
    return {'auth_mode': 'chatgpt', 'tokens': {'access_token': 'h.'+token+'.s', 'refresh_token':refresh, 'account_id':'team'}}


class CoordinatorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name).resolve()
        self.loc = dict(path=str(self.root), binary='fixture-codex', proxy=dict(mode='direct'))
        self.processes = patch.object(a, 'running', return_value=[])
        self.processes.start(); self.addCleanup(self.processes.stop)
        self.switch_processes = patch.object(a, 'switch_running', return_value=[])
        self.switch_processes.start(); self.addCleanup(self.switch_processes.stop)
        self.one, self.two = credential('one'), credential('two')
        a.atomic(self.root/'auth.json', self.one)
        a.save_profile(self.root, 'one', 'One', self.one)
        a.save_profile(self.root, 'two', 'Two', self.two)
        with a.Lock(self.root): pass

    def call(self, method, **params):
        return a.call(dict(version=1, location=self.loc, method=method, params=params))

    def test_toml_fallback_without_python_311_or_site_packages(self):
        original = builtins.__import__
        def restricted(name, *args, **kwargs):
            if name in ('tomllib', 'tomli', 'vendor'):
                raise ImportError(name)
            return original(name, *args, **kwargs)
        sources = {name: pathlib.Path(__file__).with_name('vendor').joinpath('tomli', name+'.py').read_text() for name in ('__init__','_types','_re','_parser')}
        config = self.root/'config.toml'
        config.write_text('cli_auth_credentials_store = "keyring"\n[profiles.test]\nmodel = "test"\n')
        with patch.object(a, '_aieyes_tomli_sources', sources, create=True), patch('builtins.__import__', side_effect=restricted):
            self.assertEqual(a.config(self.root)['profiles']['test']['model'], 'test')
            with self.assertRaises(ValueError): a.require_file(self.root)
            config.write_text('cli_auth_credentials_store = "file"\n')
            a.require_file(self.root)
            config.write_text('invalid = [')
            with self.assertRaises(ValueError): a.config(self.root)

    def test_lightweight_status_does_not_launch_cli_or_expose_credentials(self):
        with patch.object(a.subprocess, 'check_output', side_effect=AssertionError('status must not launch CLI')):
            result = self.call('status')
        self.assertEqual(result['currentIdentity'], a.identity(self.one)['key'])
        self.assertTrue(all(p['credential'] for p in result['profiles']))
        self.assertNotIn('refresh_token', json.dumps(result))
        self.assertNotIn('access_token', json.dumps(result))

    def test_profile_binding_list_does_not_launch_cli_or_query_processes(self):
        with patch.object(a.subprocess, 'check_output', side_effect=AssertionError('profiles must not launch CLI')), patch.object(a, 'running', side_effect=AssertionError('profiles must not query processes')):
            result = self.call('profiles.list')
        self.assertEqual({p['id'] for p in result['profiles']}, {'one', 'two'})
        self.assertEqual(result['profiles'][0]['identity']['key'], a.identity(self.one)['key'])
        self.assertNotIn('processes', result)
        self.assertNotIn('access_token', json.dumps(result))

    def test_lock_permissions_and_identity(self):
        with a.Lock(self.root):
            with self.assertRaises(ValueError):
                with a.Lock(self.root): pass
        for path in ('.aieyes', '.aieyes/accounts', '.aieyes/accounts/one'):
            self.assertEqual((self.root/path).stat().st_mode & 0o777, 0o700)
        self.assertEqual((self.root/'.aieyes/accounts/one/auth.json').stat().st_mode & 0o777, 0o600)
        self.assertNotEqual(a.identity(self.one)['key'], a.identity(self.two)['key'])
        self.assertNotIn('refresh', json.dumps(a.external(self.one)))
        with self.assertRaises(ValueError): a.save_profile(self.root,'one','Wrong',self.two)
        (self.root/'alias').symlink_to(self.root/'.aieyes/accounts/one', target_is_directory=True)
        with self.assertRaises(ValueError): a.read(self.root/'alias/auth.json')

    def test_active_authority_and_inactive_isolation(self):
        changed = credential('one','rotated')
        a.atomic(self.root/'auth.json',changed)
        self.assertEqual(a.current_auth(self.root,a.profile(self.root,'one')),changed)
        self.assertEqual(a.current_auth(self.root,a.profile(self.root,'two')),self.two)

    def test_recover_refresh_after_lost_response_and_detect_conflict(self):
        changed = credential('two','rotated')
        a.atomic(a.layout(self.root,'two','managed')/'auth.json',changed)
        journal=self.root/'.aieyes/state/refresh.json'
        a.atomic(journal,dict(profileId='two',current=False,revision=a.revision(self.two)))
        a.recover(self.root)
        self.assertFalse(journal.exists())
        self.assertEqual(a.current_auth(self.root,a.profile(self.root,'two')),changed)
        self.assertEqual(a.read(self.root/'auth.json'),self.one)
        a.atomic(a.profile_path(self.root,'two')/'auth.json',credential('two','external'))
        a.atomic(journal,dict(profileId='two',current=False,revision='external-change'))
        with self.assertRaisesRegex(ValueError,'外部修改'): a.recover(self.root)
        self.assertTrue(journal.exists())

    def test_wakeup_persists_rotation_on_inference_error(self):
        changed=credential('two','wakeup-rotation')
        with self.assertRaisesRegex(RuntimeError,'inference failed'):
            with a.with_credentials(self.loc,'two') as stage:
                a.atomic(stage/'auth.json',changed)
                raise RuntimeError('inference failed')
        self.assertEqual(a.current_auth(self.root,a.profile(self.root,'two')),changed)
        self.assertEqual(a.read(self.root/'auth.json'),self.one)
        with patch.object(a,'running',return_value=[dict(pid=123)]):
            with self.assertRaisesRegex(ValueError,'跳过唤醒'):
                with a.with_credentials(self.loc,'one'): self.fail('must not run')

    def test_switch_is_atomic_idempotent_preserves_history_and_rejects_stale(self):
        class Rpc:
            def __init__(self,*args): pass
            def __enter__(self): return self
            def __exit__(self,*args): pass
            def login(self,auth): return {}
            def call(self,*args): return {}
        history=self.root/'sessions';history.mkdir();(history/'session.jsonl').write_text('unchanged')
        op=self.call('switch.prepare',profileId='two')['operationId']
        with patch.object(a,'Rpc',Rpc):
            result=self.call('switch.commit',operationId=op)
            self.assertEqual(result['status'],'succeeded')
            self.assertEqual(self.call('switch.commit',operationId=op),result)
        self.assertEqual(a.read(self.root/'auth.json'),self.two)
        self.assertEqual((history/'session.jsonl').read_text(),'unchanged')
        op=self.call('switch.prepare',profileId='one')['operationId']
        a.atomic(self.root/'auth.json',credential('two','rotated'))
        with self.assertRaisesRegex(ValueError,'凭据'): self.call('switch.commit',operationId=op,closeProcesses=True)

    def test_external_change_during_switch_validation_aborts(self):
        root=self.root
        changed=credential('one','external-change')
        class Rpc:
            def __init__(self,*args): pass
            def __enter__(self): return self
            def __exit__(self,*args): pass
            def login(self,*args): return {}
            def call(self,*args): a.atomic(root/'auth.json',changed);return {}
        op=self.call('switch.prepare',profileId='two')['operationId']
        with patch.object(a,'Rpc',Rpc), self.assertRaisesRegex(ValueError,'验证期间'):
            self.call('switch.commit',operationId=op)
        self.assertEqual(a.read(root/'auth.json'),changed)

    def test_unknown_process_never_signalled_and_no_refresh_while_active(self):
        unknown=dict(pid=123,canClose=False,fingerprint='unknown')
        with patch.object(a,'running',return_value=[unknown]), patch.object(a.os,'kill') as kill:
            with self.assertRaises(ValueError): a.close_processes(self.root,[unknown])
            with self.assertRaisesRegex(ValueError,'运行中的 Codex'): a.renew(self.loc,self.root,a.profile(self.root,'one'))
            kill.assert_not_called()

    def test_switch_scans_chatgpt_and_other_homes_and_retains_orphans(self):
        proc = self.root / 'proc'; proc.mkdir()
        def process(pid, parent, name, start=500):
            entry = proc / str(pid); entry.mkdir(exist_ok=True)
            (entry / 'comm').write_text(name[:15])
            if not (entry / 'exe').is_symlink(): (entry / 'exe').symlink_to('/fixture/bin/' + name)
            fields = ['S', str(parent)] + ['0'] * 17 + [str(start)]
            (entry / 'stat').write_text(str(pid) + ' (' + name + ') ' + ' '.join(fields))
            (entry / 'environ').write_bytes(b'CODEX_HOME=/other-home\0')
        process(90, 1, 'ChatGPT'); process(50, 90, 'codex')
        process(20, 50, 'node'); process(10, 20, 'helper')
        process(100, 1, 'codex-code-mode-host'); process(110, 1, 'codex-unknown')
        process(120, 1, 'unrelated')
        rows = scan_switch_processes(proc_root=proc)
        self.assertEqual([(p['pid'], p['canClose']) for p in rows], [(10,True),(20,True),(50,True),(90,True),(100,True),(110,False)])
        self.assertEqual(next(p['name'] for p in rows if p['pid'] == 90), 'ChatGPT 应用')
        process(20, 1, 'node')
        self.assertNotIn(20, [p['pid'] for p in scan_switch_processes(proc_root=proc)])
        self.assertIn(20, [p['pid'] for p in scan_switch_processes(rows, proc_root=proc)])
        process(20, 1, 'node', start=600)
        self.assertNotIn(20, [p['pid'] for p in scan_switch_processes(rows, proc_root=proc)])

    def test_shared_process_profile_and_override_cases(self):
        import time
        cases = json.loads(pathlib.Path(__file__).with_name('fixtures').joinpath('codex-process-auth.json').read_text())
        for case in cases:
            with self.subTest(case=case['name']), tempfile.TemporaryDirectory() as directory:
                home = pathlib.Path(directory).resolve()
                (home / 'config.toml').write_text(case['config'])
                if case.get('changedBase'):
                    import os
                    os.utime(home / 'config.toml', (time.time()+5, time.time()+5))
                for name, text in case['profiles'].items(): (home / (name + '.config.toml')).write_text(text)
                env = dict(case['env'], CODEX_HOME=str(home))
                actual = a.process_auth_usage(case['args'], env, time.time(), system=home / 'system.toml')
                self.assertEqual(actual, case['expected'])
                if case['name'] == 'explicit_profile_inline_token':
                    self.assertEqual(a.process_auth_usage(case['args'], env, 0, system=home / 'system.toml'), 'unknownProfile')

    def test_api_profile_process_tree_is_preserved_without_secret_disclosure(self):
        import os, time
        proc = self.root / 'proc'; proc.mkdir(); (proc / 'stat').write_text('btime 0\n')
        api = json.loads(pathlib.Path(__file__).with_name('fixtures').joinpath('codex-process-auth.json').read_text())[0]['profiles']['api']
        (self.root / 'api.config.toml').write_text(api)
        ticks = int((time.time()+1) * os.sysconf('SC_CLK_TCK'))
        def process(pid, parent, name, args):
            entry = proc / str(pid); entry.mkdir()
            (entry / 'comm').write_text(name[:15]); (entry / 'exe').symlink_to('/fixture/bin/'+name)
            (entry / 'stat').write_text(str(pid)+' ('+name+') '+' '.join(['S',str(parent)]+['0']*17+[str(ticks)]))
            (entry / 'cmdline').write_bytes(('\0'.join([name]+args)+'\0').encode())
            (entry / 'environ').write_bytes(('CODEX_HOME='+str(self.root)+'\0').encode())
        process(10, 1, 'codex', ['--profile','api'])
        process(11, 10, 'codex-code-mode-host', [])
        process(12, 11, 'node', [])
        process(20, 1, 'codex', [])
        preserved = []
        rows = scan_switch_processes(proc_root=proc, preserved=preserved)
        self.assertEqual([p['pid'] for p in rows], [20])
        self.assertEqual(sorted(p['pid'] for p in preserved), [10,11,12])
        public = json.dumps(dict(processes=rows, preservedProcesses=preserved))
        self.assertNotIn('FIXTURE_SECRET', public); self.assertNotIn('fixture.invalid', public)

    def test_switch_closes_application_first_and_rejects_restarts(self):
        rows = [dict(pid=pid, parentPid=parent, fingerprint=str(pid), canClose=True) for pid,parent in [(10,20),(20,50),(50,90),(90,1)]]
        current = list(rows)
        order = []
        def terminate(pid, sig):
            order.append(pid)
            current[:] = [p for p in current if p['pid'] != pid]
        with patch.object(a, 'switch_running', side_effect=lambda *args: list(current)), patch.object(a.os, 'kill', side_effect=terminate):
            a.close_processes(self.root, rows)
        self.assertEqual(order, [90,50,20,10])
        with patch.object(a, 'switch_running', return_value=[dict(rows[0], fingerprint='reused')]), patch.object(a.os, 'kill') as kill:
            with self.assertRaisesRegex(ValueError, '重新启动'): a.close_processes(self.root, rows)
            kill.assert_not_called()
        with patch.object(a, 'switch_running', return_value=[]), patch.object(a.os, 'kill') as kill:
            a.close_processes(self.root, rows)
            kill.assert_not_called()

    def test_reauthorization_resolves_conflict_but_rejects_other_user(self):
        changed=credential('two','reauthorized')
        class Rpc:
            def __init__(self,loc,root,managed=False): self.root=root; self.managed=managed
            def __enter__(self): return self
            def __exit__(self,*args): pass
            def login(self,auth): return {}
            def receive(self,*args): return dict(method='account/login/completed',params=dict(loginId='login',success=True))
            def call(self,method,params):
                if method=='account/login/start':
                    a.atomic(self.root/'auth.json',changed)
                    return dict(loginId='login',verificationUrl='https://auth.openai.com/device',userCode='EXAMPLE')
                if method=='account/read': return dict(account=dict(type='chatgpt'))
                return {}
        journal=self.root/'.aieyes/state/refresh.json'
        a.atomic(journal,dict(profileId='two',current=False,revision='conflict'))
        with a.Lock(self.root), patch.object(a,'Rpc',Rpc):
            a.login_worker(self.loc,self.root,'reauth','Two',a.profile(self.root,'two'),a.snapshot(self.root))
        self.assertEqual(a.read(a.operation(self.root,'reauth'))['status'],'succeeded')
        self.assertFalse(journal.exists())
        self.assertEqual(a.current_auth(self.root,a.profile(self.root,'two')),changed)
        with a.Lock(self.root), patch.object(a,'Rpc',Rpc):
            a.login_worker(self.loc,self.root,'wrong','One',a.profile(self.root,'one'),a.snapshot(self.root))
        self.assertEqual(a.read(a.operation(self.root,'wrong'))['status'],'failed')
        self.assertEqual(a.read(self.root/'auth.json'),self.one)

    def test_unnamed_login_uses_email_and_accepts_omitted_name(self):
        changed = credential('new')
        token = base64.urlsafe_b64encode(json.dumps({'email':'new@example.test'}).encode()).decode().rstrip('=')
        changed['tokens']['id_token'] = 'h.' + token + '.s'
        class Rpc:
            def __init__(self,loc,root,managed=False): self.root=root
            def __enter__(self): return self
            def __exit__(self,*args): pass
            def login(self,auth): return {}
            def receive(self,*args): return dict(method='account/login/completed',params=dict(loginId='login',success=True))
            def call(self,method,params):
                if method=='account/login/start':
                    a.atomic(self.root/'auth.json',changed)
                    return dict(loginId='login',verificationUrl='https://auth.openai.com/device')
                if method=='account/read': return dict(account=dict(type='chatgpt'))
                return {}
        with patch.object(a.os, 'fork', return_value=123):
            started = self.call('login.start')
        self.assertEqual(started['status'], 'running')
        with a.Lock(self.root), patch.object(a, 'Rpc', Rpc):
            a.login_worker(self.loc,self.root,started['operationId'],'')
        result = a.read(a.operation(self.root,started['operationId']))
        self.assertEqual(result['status'], 'succeeded')
        self.assertEqual(result['result']['profile']['name'], 'new@example.test')
        self.assertEqual(a.save_profile(self.root, 'no-email', '', credential('no-email'))['name'], 'Codex 账户')

    def test_old_worker_failure_and_cancellation_are_readable(self):
        op='abandoned'
        a.atomic(a.operation(self.root,op),dict(operationId=op,status='running'))
        self.assertEqual(self.call('login.status',operationId=op)['status'],'failed')
        self.call('login.cancel',operationId=op)
        self.assertTrue((self.root/'.aieyes/state/abandoned.cancel').exists())
        with self.assertRaises(ValueError): self.call('login.cancel',operationId='../outside')

if __name__=='__main__': unittest.main()
