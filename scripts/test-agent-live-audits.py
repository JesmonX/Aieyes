#!/usr/bin/env python3
"""Offline regressions for the opt-in live scripts; never launch native clients."""
import importlib.util
import json
import pathlib
import queue
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


live = module('live', 'verify-agent-integration-live.py')
lifecycle = module('lifecycle', 'verify-agent-lifecycle-live.py')


class LiveAuditTests(unittest.TestCase):
    @staticmethod
    def acp_client():
        client = lifecycle.Acp.__new__(lifecycle.Acp)
        client.audit = types.SimpleNamespace(rpc=lambda *_: dict(sessions=[]))
        client.provider, client.max_visible = 'cline', 0
        client.phases, client.observations, client.protocol = {}, [], []
        client.permissions, client.messages = [], queue.Queue()
        client.process = types.SimpleNamespace(poll=lambda: None)
        client.send = mock.Mock()
        return client

    def test_core_and_client_environments_cannot_inherit_daily_logins_or_scans(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(live.os.environ, {
            'HOME': '/daily/home', 'USERPROFILE': '/daily/home', 'AIEYES_AUTO_SCAN': 'on',
            'CLINE_PROVIDER_SETTINGS_PATH': '/daily/providers.json', 'KILO_AUTH_CONTENT': 'PRIVATE',
            'CODEX_HOME': '/daily/codex', 'OPENAI_API_KEY': 'PRIVATE', 'HTTP_PROXY': 'PRIVATE',
        }, clear=False):
            root = pathlib.Path(directory)
            for base in (root / 'core', root / 'cline', root / 'kilo'):
                env = live.isolated_environment(base)
                self.assertEqual(env['AIEYES_AUTO_SCAN'], 'off')
                self.assertEqual(pathlib.Path(env['HOME']), base / 'user-home')
                self.assertNotIn('PRIVATE', json.dumps(env))
                self.assertNotIn('/daily/', json.dumps(env))
                self.assertNotIn('CODEX_HOME', env)
                self.assertNotIn('CLINE_PROVIDER_SETTINGS_PATH', env)

    def test_core_process_receives_the_isolated_environment(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(live.os.environ, {
            'HOME': '/daily/home', 'AIEYES_AUTO_SCAN': 'on', 'OPENAI_API_KEY': 'PRIVATE',
        }), mock.patch.object(live.subprocess, 'Popen') as popen, mock.patch.object(live.threading, 'Thread'):
            audit = live.Audit.__new__(live.Audit)
            audit.root = pathlib.Path(directory)
            audit.args = types.SimpleNamespace(core=pathlib.Path('/fake/core'))
            audit.start_core()
            kwargs = popen.call_args.kwargs
            self.assertEqual(kwargs['env']['AIEYES_AUTO_SCAN'], 'off')
            self.assertEqual(kwargs['env']['HOME'], str(audit.root / 'core/user-home'))
            self.assertNotIn('PRIVATE', json.dumps(kwargs['env']))
            kwargs['stderr'].close()

    def test_workspace_has_its_own_project_boundary_inside_another_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            outer = pathlib.Path(directory)
            subprocess.run(['git', 'init', '--quiet', str(outer)], check=True,
                           env=live.isolated_environment(outer / 'outer-env'))
            audit = live.Audit.__new__(live.Audit)
            audit.root = outer / 'audit'
            (audit.root / 'kilo').mkdir(parents=True)
            workspace = audit.workspace('kilo')
            project = subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], cwd=workspace, text=True).strip()
            self.assertEqual(pathlib.Path(project).resolve(), workspace.resolve())

    @unittest.skipUnless(hasattr(live.os, 'killpg'), 'POSIX process-group cleanup')
    def test_cleanup_escalates_when_the_native_process_ignores_term(self):
        process = mock.Mock(pid=98765)
        process.poll.return_value = None
        process.wait.side_effect = [subprocess.TimeoutExpired('isolated-native', 5), 0]
        with mock.patch.object(live.os, 'killpg') as killpg:
            live.stop_process(process, group=True)
        self.assertEqual(killpg.call_args_list, [mock.call(98765, live.signal.SIGTERM), mock.call(98765, live.signal.SIGKILL)])
        self.assertEqual(process.wait.call_count, 2)

    def test_cleanup_escalates_for_an_individual_process(self):
        process = mock.Mock()
        process.poll.return_value = None
        process.wait.side_effect = [subprocess.TimeoutExpired('isolated-native', 5), 0]
        live.stop_process(process)
        process.terminate.assert_called_once_with()
        process.kill.assert_called_once_with()
        self.assertEqual(process.wait.call_count, 2)

    def test_detection_and_unsupported_surfaces_are_never_claimed_live_verified(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(live.shutil, 'which', return_value=None):
            home = pathlib.Path(directory)
            (home / '.vscode/extensions/rooveterinaryinc.roo-cline-3.54.0').mkdir(parents=True)
            (home / '.vscode/extensions/github.copilot-chat-1.0.0').mkdir(parents=True)
            roo = live.availability('roo', live.discover('roo', home=home))
            self.assertEqual(roo['status'], 'unverified')
            self.assertFalse(roo['liveVerified'])
            copilot_extension = live.availability('copilot', live.discover('copilot', home=home))
            self.assertEqual(copilot_extension['status'], 'unverified')
            self.assertTrue(copilot_extension['discovery']['installed'])
            missing = live.availability('cline', live.discover('cline', home=home))
            self.assertEqual(missing['status'], 'skipped')
            self.assertEqual(missing['reason'], 'notInstalled')
            ready = live.availability('cline', live.discover('cline', {'cline': '/isolated/bin/cline'}, home))
            self.assertEqual(ready['status'], 'ready')
            self.assertFalse(ready['liveVerified'])
            self.assertIn('kilo', live.provider_list('auto'))
            self.assertIn('trae-cn', live.provider_list('auto'))

    def test_copilot_uses_documented_offline_byok_without_native_credentials(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(live.os.environ, {
            'COPILOT_GITHUB_TOKEN': 'PRIVATE', 'GH_TOKEN': 'PRIVATE', 'GITHUB_TOKEN': 'PRIVATE',
            'COPILOT_PROVIDER_API_KEY_COMMAND': 'PRIVATE', 'COPILOT_PROVIDER_HEADERS': 'PRIVATE',
        }):
            audit = live.Audit.__new__(live.Audit)
            audit.root = pathlib.Path(directory)
            audit.sources = {'copilot': audit.root / 'copilot/home'}
            audit.sources['copilot'].mkdir(parents=True)
            audit.args = types.SimpleNamespace(endpoint='http://127.0.0.1:11444', model='audit-model')
            audit.server = types.SimpleNamespace(server_address=('127.0.0.1', 54321))
            audit.program = lambda _: '/isolated/copilot'
            config = audit.configure('copilot')
            env = audit.environment('copilot', config)
            self.assertEqual(env['COPILOT_HOME'], str(audit.sources['copilot']))
            self.assertEqual(env['COPILOT_PROVIDER_BASE_URL'], 'http://127.0.0.1:54321/copilot/v1')
            self.assertEqual(env['COPILOT_PROVIDER_WIRE_API'], 'completions')
            self.assertEqual(env['COPILOT_PROVIDER_API_KEY'], 'not-required')
            self.assertEqual(env['COPILOT_OFFLINE'], 'true')
            self.assertNotIn('PRIVATE', json.dumps(env))
            command = audit.command('copilot', 'Read the audit marker', tool=True, resume=True)
            for flag in ('--continue', '--disable-builtin-mcps', '--no-custom-instructions', '--no-bash-env', '--sync-session=off'):
                self.assertIn(flag, command)
            self.assertNotIn('--trust-folder', command)
            self.assertIn('--trust-folder', audit.command('copilot', 'Read the audit marker'))
            detected = dict(installed=True, program='/isolated/copilot', surfaces=[])
            self.assertEqual(live.availability('copilot', detected)['status'], 'ready')
            self.assertEqual(live.availability('copilot', detected, lifecycle=True)['status'], 'unverified')

    def test_headless_success_requires_usage_and_exclusion_without_visible_complete(self):
        case = dict(exitCode=0, marker=True, usageMatches=True, httpTokens=10, headlessExcluded=True,
                    observations=[], calls=[dict(status=200)])
        self.assertTrue(live.case_passed(case))
        self.assertFalse(live.case_passed(dict(case, headlessExcluded=False)))
        self.assertFalse(live.case_passed(dict(case, httpTokens=0)))
        self.assertFalse(live.case_passed(dict(case, usageMatches=False)))
        self.assertFalse(live.case_passed(dict(case, maxVisibleSessions=1)))

    def test_console_summary_names_main_and_auxiliary_usage_and_preserves_the_report(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch('builtins.print') as printed:
            audit = live.Audit.__new__(live.Audit)
            audit.root = pathlib.Path(directory)
            audit.args = types.SimpleNamespace(endpoint='http://127.0.0.1:11444', model='audit-model')
            reason = 'ACP session appeared in the visible session list'
            audit.results = {'codebuddy': dict(status='failed', permission=dict(
                importedTokens=46959, mainHttpTokens=46959, httpTokens=47544,
                maxVisibleSessions=1, observations=[], status='failed', reason=reason))}
            audit.save()
            summary = json.loads(printed.call_args.args[0])['codebuddy']
            self.assertEqual(summary['tokens']['permission'], dict(
                importedTokens=46959, mainHttpTokens=46959, auxiliaryHttpTokens=585))
            self.assertEqual(summary['caseReasons'], {'permission': reason})
            self.assertEqual(json.loads((audit.root / 'report.json').read_text())['results'], audit.results)
            self.assertEqual(lifecycle.case_status('permission', audit.results['codebuddy']['permission'])[0], 'failed')

    def test_partial_integration_coverage_is_never_live_verified_and_strict_mode_fails(self):
        case = dict(exitCode=0, marker=True, usageMatches=True, httpTokens=10, headlessExcluded=True,
                    observations=[], calls=[dict(status=200)])
        for strict, expected in ((False, 0), (True, 1)):
            results = {'cline': dict(status='ready', liveVerified=False)}
            audit = mock.Mock()
            audit.run_case.return_value = case
            audit.verify_replay.return_value = dict(stable=True)
            argv = ['live-audit', '--providers', 'cline', '--cases', 'normal,multiTurn']
            if strict:
                argv.append('--require-live')
            with mock.patch.object(sys, 'argv', argv), mock.patch.object(live, 'prepare', return_value=(results, ['cline'])), \
                    mock.patch.object(live, 'Audit', return_value=audit), mock.patch('builtins.print'):
                with self.assertRaises(SystemExit) as exited:
                    live.main()
            self.assertEqual(exited.exception.code, expected)
            self.assertEqual(results['cline']['status'], 'partial')
            self.assertFalse(results['cline']['liveVerified'])
            self.assertEqual(results['cline']['unverifiedCases'], ['multiTurn'])
            self.assertEqual(results['cline']['verifiedCases'], ['normal'])

    def test_a_standalone_multi_turn_case_creates_a_seed_before_resuming(self):
        results = {'copilot': dict(status='ready', liveVerified=False)}
        audit = mock.Mock()
        audit.run_case.return_value = dict(exitCode=0, marker=True, usageMatches=True, httpTokens=10,
                                           headlessExcluded=True, observations=[], calls=[dict(status=200)])
        audit.verify_replay.return_value = dict(stable=True)
        with mock.patch.object(sys, 'argv', ['live-audit', '--providers', 'copilot', '--cases', 'multiTurn']), \
                mock.patch.object(live, 'prepare', return_value=(results, ['copilot'])), \
                mock.patch.object(live, 'Audit', return_value=audit), mock.patch('builtins.print'):
            with self.assertRaises(SystemExit) as exited:
                live.main()
        self.assertEqual(exited.exception.code, 0)
        calls = audit.run_case.call_args_list
        self.assertEqual([call.args[2] for call in calls], ['multiTurnSeed', 'multiTurn'])
        self.assertTrue(calls[1].kwargs['resume'])

    def test_acp_cancel_is_triggered_by_protocol_while_aieyes_has_no_visible_sessions(self):
        client = self.acp_client()
        sent = []
        client.send = sent.append
        client.messages.put(dict(method='session/update', params=dict(update=dict(
            sessionUpdate='tool_call_update', status='in_progress', kind='execute', title='PRIVATE', content='PRIVATE'))))
        client.messages.put(dict(id='request', result=dict(stopReason='cancelled')))
        response, cancelled = client.receive('request', 'session', cancel=True)
        self.assertTrue(cancelled)
        self.assertEqual(response['stopReason'], 'cancelled')
        self.assertEqual(sent, [dict(jsonrpc='2.0', method='session/cancel', params=dict(sessionId='session'))])
        self.assertEqual(client.max_visible, 0)
        self.assertNotIn('PRIVATE', json.dumps(client.protocol))

    def test_native_authentication_requirement_remains_unverified_without_login(self):
        client = self.acp_client()
        client.messages.put(dict(id='request', error=dict(code=-32000, message='Authentication required: Call authenticate before starting a session')))
        with self.assertRaisesRegex(lifecycle.UnverifiedLiveCase, 'native account authentication'):
            client.receive('request')
        client.send.assert_not_called()

    def test_acp_timeout_preserves_child_evidence_and_marks_remaining_cases_unverified(self):
        client = self.acp_client()
        client.provider, client.workspace = 'kilo', pathlib.Path('/isolated/workspace')
        client.audit = types.SimpleNamespace(args=types.SimpleNamespace(cases='child,cancel'), results={'kilo': {}},
                                             calls=[], totals=lambda _: dict(total=0), rpc=lambda *_: [],
                                             save=mock.Mock(), verify_replay=lambda _: dict(stable=True), wire_total=live.Audit.wire_total)
        client.snapshot = mock.Mock()
        client.request = mock.Mock(return_value='request')
        client.native_children = lambda: 1
        client.receive = mock.Mock(side_effect=[(dict(authMethods=[]), False), (dict(sessionId='session'), False),
                                                lifecycle.UnverifiedLiveCase('Native request timed out')])
        with mock.patch.object(lifecycle.native, 'stop_process'), mock.patch.object(lifecycle.time, 'monotonic', side_effect=[0, 4]):
            result = client.run()
        self.assertEqual(result['child']['nativeChildren'], 1)
        self.assertEqual(result['child']['status'], 'unverified')
        self.assertEqual(result['cancel']['status'], 'unverified')
        self.assertIn('timed out', result['child']['reason'])

    def test_sending_cancel_without_a_native_acknowledgement_is_not_success(self):
        case = dict(maxVisibleSessions=0, observations=[], importedTokens=10, mainHttpTokens=10,
                    calls=[dict(status=200)], cancelled=True, stopReason='end_turn')
        self.assertFalse(lifecycle.case_passed('cancel', case))
        self.assertTrue(lifecycle.case_passed('cancel', dict(case, stopReason='cancelled')))
        self.assertFalse(lifecycle.case_passed('cancel', dict(case, stopReason='cancelled', maxVisibleSessions=1)))

    def test_missing_native_cancel_usage_is_unverified_and_other_mismatches_fail(self):
        case = dict(maxVisibleSessions=0, observations=[], importedTokens=0, mainHttpTokens=10,
                    calls=[dict(status=200)], cancelled=True, stopReason='cancelled',
                    nativeUsage=dict(lastAssistantError='MessageAbortedError', lastAssistantTokens=0))
        self.assertEqual(lifecycle.case_status('cancel', case)[0], 'unverified')
        self.assertEqual(lifecycle.case_status('cancel', dict(case, nativeUsage={}))[0], 'failed')


if __name__ == '__main__':
    unittest.main()
