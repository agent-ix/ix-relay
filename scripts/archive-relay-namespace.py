#!/usr/bin/env python3
"""Read-only, binary-safe Redis namespace archive. Never imports old identities."""
import argparse
import base64
import json
import os
from pathlib import Path
import socket
import ssl
import time
import tomllib
from urllib.parse import unquote, urlparse


class Redis:
    def __init__(self, url):
        endpoint = urlparse(url)
        if endpoint.scheme not in {'redis', 'rediss'}:
            raise ValueError('expected a Redis URL')
        connection = socket.create_connection((endpoint.hostname, endpoint.port or 6379), timeout=10)
        if endpoint.scheme == 'rediss':
            connection = ssl.create_default_context().wrap_socket(connection, server_hostname=endpoint.hostname)
        self.socket = connection
        self.reader = connection.makefile('rb')
        if endpoint.password:
            auth = [unquote(endpoint.password)]
            if endpoint.username:
                auth.insert(0, unquote(endpoint.username))
            self.command('AUTH', *auth)
        if endpoint.path not in {'', '/'}:
            self.command('SELECT', endpoint.path.lstrip('/'))

    def command(self, *args):
        encoded = [arg if isinstance(arg, bytes) else str(arg).encode() for arg in args]
        request = b'*' + str(len(encoded)).encode() + b'\r\n'
        for arg in encoded:
            request += b'$' + str(len(arg)).encode() + b'\r\n' + arg + b'\r\n'
        self.socket.sendall(request)
        return self.reply()

    def reply(self):
        line = self.reader.readline(4096)
        if not line.endswith(b'\r\n'):
            raise ValueError('incomplete Redis reply')
        kind, value = line[:1], line[1:-2]
        if kind == b'-':
            raise ValueError('Redis rejected the archive read')
        if kind == b'+':
            return value
        if kind == b':':
            return int(value)
        size = int(value)
        if size == -1:
            return None
        if kind == b'$' and 0 <= size <= 512 * 1024 * 1024:
            data = self.reader.read(size)
            if len(data) != size or self.reader.read(2) != b'\r\n':
                raise ValueError('incomplete Redis bulk reply')
            return data
        if kind == b'*' and 0 <= size <= 100000:
            return [self.reply() for _ in range(size)]
        raise ValueError('unsupported Redis reply')

    def close(self):
        self.reader.close()
        self.socket.close()


def archive(url, prefix, output):
    if not prefix or prefix == 'ixr:v3:':
        raise ValueError('choose the explicit prior namespace, not the new v3 namespace')
    connection = Redis(url)
    count = 0
    seen = set()
    # Exclusive creation avoids overwriting any existing backlog archive.
    try:
        fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except OSError:
        connection.close()
        raise
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as destination:
            destination.write(json.dumps({'format': 'redis-dump-jsonl-v1', 'prefix': prefix,
                'created_at_ms': int(time.time() * 1000), 'consistent_snapshot': False}) + '\n')
            cursor = b'0'
            while True:
                cursor, keys = connection.command('SCAN', cursor, 'MATCH', prefix + '*', 'COUNT', 100)
                for key in keys:
                    if key.startswith(b'ixr:v3:') or key in seen:
                        continue
                    seen.add(key)
                    ttl = connection.command('PTTL', key)
                    value = connection.command('DUMP', key)
                    if value is None or ttl == -2:
                        continue
                    destination.write(json.dumps({'key_base64': base64.b64encode(key).decode(),
                        'dump_base64': base64.b64encode(value).decode(), 'ttl_ms': ttl,
                        'captured_at_ms': int(time.time() * 1000)}) + '\n')
                    count += 1
                if cursor == b'0':
                    break
            destination.flush()
            os.fsync(destination.fileno())
    finally:
        connection.close()
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prefix', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--config', type=Path, default=Path.home() / '.ix/relay/config.toml')
    args = parser.parse_args()
    url = os.environ.get('IX_RELAY_URL')
    if not url:
        url = tomllib.loads(args.config.read_text())['url']
    print(f'archived {archive(url, args.prefix, args.output)} keys; source keys unchanged')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError):
        raise SystemExit('archive failed; source keys unchanged; inspect any partial output before retrying') from None
