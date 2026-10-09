#!/usr/bin/env python3
"""Offline coordinator fault tests. No real accounts or processes are touched."""
import base64
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('auth_helper', pathlib.Path(__file__).with_name('remote_codex_auth.py'))
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)


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
        self.one, self.two = credential('one'), credential('two')
        a.atomic(self.root/'auth.json', self.one)
        a.save_profile(self.root, 'one', 'One', self.one)
        a.save_profile(self.root, 'two', 'Two', self.two)
        with a.Lock(self.root): pass

    def call(self, method, **params):
        return a.call(dict(version=1, location=self.loc, method=method, params=params))

    def test_lightweight_status_does_not_launch_cli_or_expose_credentials(self):
        with patch.object(a.subprocess, 'check_output', side_effect=AssertionError('status must not launch CLI')):
            result = self.call('status')
        self.assertEqual(result['currentIdentity'], a.identity(self.one)['key'])
        self.assertTrue(all(p['credential'] for p in result['profiles']))
        self.assertNotIn('refresh_token', json.dumps(result))
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
