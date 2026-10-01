"""
Test harness for capd, exercised exclusively over its Unix domain socket
control channel. Each test starts a fresh capd subprocess with its own
isolated RUNTIME_DIRECTORY, so tests never interfere with each other or
with a real systemd-managed instance running elsewhere on the machine.

Run with:  pytest -v tests/test_capd.py
"""
import os
import socket
import subprocess
import time
import pytest

CAPD_BINARY = os.environ.get("CAPD_BINARY", "./target/debug/capd")


@pytest.fixture
def capd(tmp_path):
    """Starts a capd instance with RUNTIME_DIRECTORY pointed at a fresh
    pytest tmp_path, waits for it to actually be ready, yields the socket
    path, then shuts it down gracefully and asserts clean exit."""
    env = os.environ.copy()
    env["RUNTIME_DIRECTORY"] = str(tmp_path)

    proc = subprocess.Popen(
        [CAPD_BINARY],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )

    sock_path = str(tmp_path / "capd.sock")

    # Poll for the socket file to appear rather than a fixed sleep --
    # faster on a quiet machine, more reliable on a loaded one (e.g. CI).
    deadline = time.time() + 5
    while not os.path.exists(sock_path):
        if proc.poll() is not None:
            out = proc.stdout.read()
            pytest.fail(f"capd exited early (code={proc.returncode}):\n{out}")
        if time.time() > deadline:
            proc.kill()
            pytest.fail("capd never created its control socket within 5s")
        time.sleep(0.02)

    yield sock_path

    # Graceful shutdown: SIGTERM, same as `systemctl stop` would send.
    proc.terminate()
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        pytest.fail("capd did not exit within 3s of SIGTERM — graceful shutdown broken")

    assert proc.returncode == 0, (
        f"capd should exit 0 on SIGTERM, got {proc.returncode}"
    )


def send_command(sock_path: str, command: str) -> str:
    """Connects fresh for each command, exactly like `nc -U` does --
    exercises the real accept()/per-client path in capd's epoll loop,
    not just a single long-lived connection."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.settimeout(2)
        s.connect(sock_path)
        s.sendall((command + "\n").encode())
        return s.recv(4096).decode()


def test_status_starts_at_zero_or_counts_up(capd):
    reply = send_command(capd, "status")
    assert reply.startswith("STATUS:"), f"unexpected reply: {reply!r}"
    assert "packets captured" in reply

def test_reset_command_replies_ok(capd):
    reply = send_command(capd, "reset")
    assert reply == "OK: counter reset\n", f"unexpected reply: {reply!r}"

def test_unknown_command_gets_error_reply(capd):
    reply = send_command(capd, "not_a_real_command")
    assert reply.startswith("ERR:"), f"unexpected reply: {reply!r}"
    assert "not_a_real_command" in reply


def test_multiple_sequential_clients_all_get_replies(capd):
    # Proves the daemon correctly handles connect -> command -> disconnect,
    # repeated, rather than only working for a single persistent client.
    for _ in range(5):
        reply = send_command(capd, "status")
        assert reply.startswith("STATUS:")


def test_shutdown_cleanup_standalone(tmp_path):
    """Self-contained variant (doesn't use the `capd` fixture) so we can
    inspect filesystem state AFTER shutdown, which the shared fixture's
    teardown timing makes awkward to assert from within a normal test."""
    env = os.environ.copy()
    env["RUNTIME_DIRECTORY"] = str(tmp_path)
    sock_path = tmp_path / "capd.sock"

    proc = subprocess.Popen([CAPD_BINARY], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    deadline = time.time() + 5
    while not sock_path.exists():
        if proc.poll() is not None:
            pytest.fail(f"capd exited early:\n{proc.stdout.read()}")
        if time.time() > deadline:
            proc.kill()
            pytest.fail("capd never started")
        time.sleep(0.02)

    proc.terminate()
    proc.wait(timeout=3)

    assert not sock_path.exists(), "socket file should be removed after graceful shutdown"
    assert not (tmp_path / "capd.ctl").exists(), "FIFO should be removed after graceful shutdown"