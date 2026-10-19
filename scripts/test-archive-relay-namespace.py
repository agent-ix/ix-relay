#!/usr/bin/env python3
"""Archive binary keys/values and TTL without mutating or replacing source data."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
spec = importlib.util.spec_from_file_location('archive', Path(__file__).with_name('archive-relay-namespace.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

@unittest.skipUnless(os.environ.get('IX_RELAY_TEST_URL'), 'requires isolated test Redis')
class ArchiveTests(unittest.TestCase):
    def test_binary_dump_ttl_source_preservation_and_exclusive_output(self):
        url = os.environ['IX_RELAY_TEST_URL']
        connection = module.Redis(url)
        prefix = f'test:archive:{os.getpid()}:'
        key = prefix.encode() + b'\xff'
        connection.command('SET',key,b'\x00\r\n\xff', 'PX',60000)
        try:
            with tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / 'archive.jsonl'
                self.assertEqual(module.archive(url,prefix,output),1)
                rows = [json.loads(line) for line in output.read_text().splitlines()]
                self.assertEqual(rows[0]['prefix'],prefix)
                self.assertGreater(rows[1]['ttl_ms'],0)
                self.assertEqual(connection.command('GET',key),b'\x00\r\n\xff')
                self.assertEqual(output.stat().st_mode & 0o777,0o600)
                with self.assertRaises(FileExistsError):
                    module.archive(url,prefix,output)
        finally:
            connection.command('DEL',key)
            connection.close()

if __name__ == '__main__':
    unittest.main()
