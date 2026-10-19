#!/usr/bin/env python3
"""Launcher regression tests; no Codex session or account access."""
import importlib.util
import pathlib
import unittest
spec = importlib.util.spec_from_file_location('launcher', pathlib.Path(__file__).with_name('codex-herdr-launcher.py'))
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)

class LauncherTests(unittest.TestCase):
    def test_interactive_context_and_existing_environment(self):
        env = {'HERDR_ENV': '1', 'AGENT_MSG_NAME': 'leave-this-alone'}
        for args in [[], ['resume', 'session-id'], ['fork', '--last'], ['--model', 'gpt-6', 'a prompt'], ['--model', 'exec'], ['--config', 'x=exec', 'resume', 'session-id']]:
            self.assertEqual(launcher.arguments(args, env), ['--no-daemon', *args])
        self.assertEqual(env['AGENT_MSG_NAME'], 'leave-this-alone')
    def test_noninteractive_and_outside_herdr_pass_through(self):
        for args in [['--model', 'gpt-6', 'exec', 'prompt'], ['exec', 'prompt'], ['--version'], ['mcp', 'list'], ['resume', '--no-daemon', 'session']]:
            self.assertEqual(launcher.arguments(args, {'HERDR_ENV': '1'}), args)
        self.assertEqual(launcher.arguments(['resume', 'session'], {}), ['resume', 'session'])

if __name__ == '__main__':
    unittest.main()
