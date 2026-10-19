#!/usr/bin/env python3
"""Keep interactive Herdr Codex sessions in their own terminal context."""
import os
import pathlib
import sys


def arguments(args, env):
    """Preserve CLI commands; isolate only interactive starts/resumes/forks."""
    noninteractive = {
        'exec', 'e', 'review', 'login', 'logout', 'mcp', 'mcp-server',
        'app-server', 'app-server-daemon', 'completion', 'sandbox', 'debug',
        'apply', 'a', 'cloud', 'features', 'help', 'version', 'update',
    }
    # Skip option values before deciding whether the first positional token is
    # a CLI subcommand. A model or profile named "exec" is still interactive.
    value_options = {'--config', '-c', '--model', '-m', '--profile', '-p',
                     '--sandbox', '-s', '--ask-for-approval', '-a', '--cd', '-C',
                     '--image', '-i', '--add-dir', '--enable', '--disable'}
    first = None
    skip_value = False
    for arg in args:
        if skip_value:
            skip_value = False
            continue
        if arg == '--':
            break
        if arg in value_options:
            skip_value = True
        elif not arg.startswith('-'):
            first = arg
            break
    informational = any(arg in {'--help', '-h', '--version', '-V'} for arg in args)
    if env.get('HERDR_ENV') == '1' and not informational and first not in noninteractive and '--no-daemon' not in args:
        return ['--no-daemon', *args]
    return args


def main():
    own = pathlib.Path(__file__).resolve()
    for directory in os.environ.get('PATH', '').split(os.pathsep):
        candidate = pathlib.Path(directory) / 'codex'
        if candidate.is_file() and os.access(candidate, os.X_OK) and candidate.resolve() != own:
            os.execv(str(candidate), [str(candidate), *arguments(sys.argv[1:], os.environ)])
    sys.exit('codex: the Herdr launcher could not find the underlying Codex executable on PATH')


if __name__ == '__main__':
    main()
