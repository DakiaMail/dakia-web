#!/usr/bin/env python3
"""Run real Dovecot integration in a disposable, loopback-only container.

No owner credentials or mailbox paths are consumed. The stopped fixture container
is retained for inspection; normal Docker cleanup can remove it later.
"""
from pathlib import Path
import argparse
import json
import os
import signal
import socket
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
IMAGE = "dovecot/dovecot@sha256:547d28f9a893c93a1d2f6dc164880cd7bfa74009106dfa46b36a6a2ee636d921"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def remote_fixture(host, name, directory):
    """Run synthetic amd64 Dovecot over SSH when local Docker cannot emulate it.

    Only generated certificates and the checked-in synthetic config cross SSH.
    Both remote listeners and the local forwarded listeners bind to loopback.
    """
    if host.startswith("-"):
        raise ValueError("SSH host must not be an option")
    payload = json.dumps({"name": name, "image": IMAGE,
        "config": (ROOT / "fixtures/dovecot/dovecot.conf").read_text(),
        "cert": (directory / "cert.pem").read_text(),
        "key": (directory / "key.pem").read_text()})
    startup = '''import json, pathlib, subprocess, sys, tempfile
p = json.load(sys.stdin)
d = pathlib.Path(tempfile.mkdtemp(prefix="dakia-provider-fixture-"))
for name, data in [("dovecot.conf", p["config"]), ("cert.pem", p["cert"]), ("key.pem", p["key"])]:
    (d / name).write_text(data)
    (d / name).chmod(0o600)
try:
    subprocess.run(["docker", "run", "-d", "--name", p["name"], "-p", "127.0.0.1::143", "-p", "127.0.0.1::993", "--mount", f"type=bind,src={d}/dovecot.conf,dst=/etc/dovecot/dovecot.conf,readonly", "--mount", f"type=bind,src={d},dst=/fixture,readonly", p["image"]], check=True, stdout=subprocess.DEVNULL)
    state = json.loads(subprocess.check_output(["docker", "inspect", p["name"]]))[0]
    subprocess.run(["docker", "exec", p["name"], "doveadm", "service", "status", "imap-login"], check=True, stdout=subprocess.DEVNULL)
    print(json.dumps({"ports": state["NetworkSettings"]["Ports"], "directory": str(d)}))
except BaseException:
    subprocess.run(["docker", "stop", "--time", "2", p["name"]], stdout=subprocess.DEVNULL)
    raise
'''
    # Pass code as a shell-quoted command argument, data only on standard input.
    import shlex
    state = json.loads(run("ssh", "-o", "BatchMode=yes", host,
        "python3 -c " + shlex.quote(startup), input=payload, capture_output=True).stdout)
    tunnel = None
    try:
        forwards, ports = [], {}
        for remote in (143, 993):
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", 0))
                local = probe.getsockname()[1]
            ports[remote] = str(local)
            forwards += ["-L", f"127.0.0.1:{local}:127.0.0.1:{state['ports'][str(remote) + '/tcp'][0]['HostPort']}"]
        tunnel = subprocess.Popen(["ssh", "-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes", *forwards, host])
        for _ in range(100):
            if tunnel.poll() is not None:
                raise RuntimeError("fixture SSH tunnel exited")
            try:
                with socket.create_connection(("127.0.0.1", int(ports[993])), timeout=0.1):
                    break
            except OSError:
                time.sleep(0.1)
        else:
            raise TimeoutError("fixture SSH tunnel did not open")
        env = dict(os.environ, DAKIA_FIXTURE_IMAPS_PORT=ports[993], DAKIA_FIXTURE_IMAP_PORT=ports[143], DAKIA_FIXTURE_CERT=str(directory / "cert.pem"))
        run("cargo", "test", "--manifest-path", str(ROOT / "Cargo.toml"), "--test", "dovecot", "--", "--ignored", env=env)
    finally:
        if tunnel:
            tunnel.terminate()
            tunnel.wait(timeout=10)
        run("ssh", "-o", "BatchMode=yes", host, "docker stop --time 2 " + shlex.quote(name), capture_output=True)
        print(f"Stopped synthetic fixture container {name} on {host}; fixture files retained at {state['directory']}")


def main():
    def terminate(signum, _frame):
        raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, terminate)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ssh-host", help="Optional SSH host with native amd64 Docker, for synthetic fixtures only")
    args = parser.parse_args()
    name = "dakia-provider-fixture-" + uuid.uuid4().hex[:10]
    directory = ROOT / "target" / name
    directory.mkdir(parents=True)
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
        "-keyout", str(directory / "key.pem"), "-out", str(directory / "cert.pem"),
        "-days", "2", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost",
        "-addext", "extendedKeyUsage=serverAuth",
        capture_output=True)
    if args.ssh_host:
        return remote_fixture(args.ssh_host, name, directory)
    run("docker", "run", "-d", "--name", name, "-p", "127.0.0.1::143", "-p", "127.0.0.1::993",
        "--mount", f"type=bind,src={ROOT / 'fixtures/dovecot/dovecot.conf'},dst=/etc/dovecot/dovecot.conf,readonly",
        "--mount", f"type=bind,src={directory},dst=/fixture,readonly", IMAGE, capture_output=True)
    try:
        state = json.loads(run("docker", "inspect", name, capture_output=True).stdout)[0]
        ports = state["NetworkSettings"]["Ports"]
        env = dict(os.environ, DAKIA_FIXTURE_IMAPS_PORT=ports["993/tcp"][0]["HostPort"],
                   DAKIA_FIXTURE_IMAP_PORT=ports["143/tcp"][0]["HostPort"],
                   DAKIA_FIXTURE_CERT=str(directory / "cert.pem"))
        # Verify Dovecot config/startup rather than an arbitrary sleep.
        run("docker", "exec", name, "doveadm", "service", "status", "imap-login", capture_output=True)
        run("cargo", "test", "--manifest-path", str(ROOT / "Cargo.toml"), "--test", "dovecot", "--", "--ignored", env=env)
    finally:
        run("docker", "stop", "--time", "2", name, capture_output=True)
        print(f"Stopped fixture container: {name}")


if __name__ == "__main__":
    main()
