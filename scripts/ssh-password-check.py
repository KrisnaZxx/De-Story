"""Verify the real desktop executable's askpass with a loopback-only fake SSH server.
Requires paramiko in the test environment and the ignored native fixture writer.
"""
import argparse
import os
import pathlib
import socket
import subprocess
import threading
import paramiko

parser = argparse.ArgumentParser()
parser.add_argument('--exe', required=True)
parser.add_argument('--fixture', required=True)
args = parser.parse_args()
fixture = pathlib.Path(args.fixture).resolve()
assert fixture.name.startswith('ssh-password-check-') and fixture.parent.name == '.toolchain'
key = paramiko.RSAKey.generate(2048)
listener = socket.socket()
listener.bind(('127.0.0.1', 0))
listener.listen(1)
listener.settimeout(45)
port = listener.getsockname()[1]
failures = []


class Server(paramiko.ServerInterface):
    def check_auth_password(self, username, password):
        return paramiko.AUTH_SUCCESSFUL if username == 'demo' and password == 'fixture password only' else paramiko.AUTH_FAILED

    def get_allowed_auths(self, username):
        return 'password'

    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == 'session' else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_exec_request(self, channel, command):
        channel.send(b'password-auth-ok\n')
        channel.send_exit_status(0)
        channel.shutdown_write()
        return True


def serve():
    try:
        sock, _ = listener.accept()
        with paramiko.Transport(sock) as transport:
            transport.add_server_key(key)
            transport.start_server(server=Server())
            channel = transport.accept(20)
            if channel:
                channel.settimeout(20)
                channel.recv(1)
                channel.close()
    except Exception as exc:
        failures.append(type(exc).__name__)
    finally:
        listener.close()


known = fixture / 'known_hosts'
known.write_text(f'[127.0.0.1]:{port} {key.get_name()} {key.get_base64()}\n')
config = fixture / 'config'
config.write_text('')
environment = dict(os.environ, STUDIO_USER_DATA=str(fixture), SSH_ASKPASS=str(pathlib.Path(args.exe).resolve()),
                   SSH_ASKPASS_REQUIRE='force', DE_STORY_SSH_ASKPASS_PROFILE='fixture-password')
helper = subprocess.run([args.exe, "demo@127.0.0.1's password:"], env=environment, capture_output=True, timeout=20)
assert helper.returncode == 0 and helper.stdout.strip() == b'fixture password only', 'Askpass helper did not return the fixture credential'
otp = subprocess.run([args.exe, 'Verification code:'], env=environment, capture_output=True, timeout=20)
assert otp.returncode != 0 and not otp.stdout, 'Additional verification must not receive the SSH password'
worker = threading.Thread(target=serve, daemon=True)
worker.start()
result = subprocess.run(['ssh', '-F', str(config), '-o', f'UserKnownHostsFile={known}', '-o', 'StrictHostKeyChecking=yes',
                         '-o', 'BatchMode=no', '-o', 'NumberOfPasswordPrompts=1', '-o', 'PreferredAuthentications=password',
                         '-o', 'PubkeyAuthentication=no', '-p', str(port), 'demo@127.0.0.1', 'echo ok'],
                        env=environment, capture_output=True, timeout=30)
worker.join(timeout=5)
assert result.returncode == 0 and b'password-auth-ok' in result.stdout, 'OpenSSH password authentication failed: ' + result.stderr.decode(errors='replace')
print('Encrypted askpass and real OpenSSH password login passed; OTP prompts rejected; no real VPS credentials used.')
