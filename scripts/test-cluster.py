#!/usr/bin/env python3
"""Exercise labadm with real etcd quorum, TLS, and simulated OpenSDL workers."""

import argparse
import base64
import concurrent.futures
import contextlib
import json
import os
import shutil
import select
import socket
import socketserver
import sqlite3
import ssl
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import uuid
from pathlib import Path

IMAGE = "quay.io/coreos/etcd:v3.6.5"
LOOPBACK = "127.0.0.1"
DEVICE = "sim:lab-sim:heater-1"


def run(*args, env=None):
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=40, env=env).stdout.strip()


LABADM_ENV = {name: value for name, value in os.environ.items() if not name.upper().startswith("OSDL_")}


def wait_for(probe, timeout=30):
    end = time.monotonic() + timeout
    while True:
        try:
            result = probe()
            if result:
                return result
        except (OSError, subprocess.CalledProcessError) as error:
            if time.monotonic() >= end:
                raise RuntimeError(f"timed out waiting for {probe}: {error.stderr if isinstance(error, subprocess.CalledProcessError) else error}") from error
        if time.monotonic() >= end:
            raise RuntimeError(f"timed out waiting for {probe}")
        time.sleep(0.15)


def ports(count):
    with contextlib.ExitStack() as stack:
        listeners = [stack.enter_context(socket.socket()) for _ in range(count)]
        for listener in listeners:
            listener.bind((LOOPBACK, 0))
        return [listener.getsockname()[1] for listener in listeners]


class Pki:
    def __init__(self, root, openssl):
        self.root = root
        self.openssl = openssl
        for role in ("operator", "control", "worker", "etcd"):
            run(
                openssl, "req", "-x509", "-newkey", "ec",
                "-pkeyopt", "ec_paramgen_curve:P-256", "-noenc", "-days", "2",
                "-subj", f"/CN=osdl-{role}-qa", "-keyout", self.file(f"{role}-ca.key"),
                "-out", self.file(f"{role}-ca.crt"),
                "-addext", "basicConstraints=critical,CA:TRUE",
                "-addext", "keyUsage=critical,keyCertSign,cRLSign",
            )

    def file(self, name):
        return self.root / name

    def issue(self, name, role, usage, san=None):
        run(
            self.openssl, "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
            "-noenc", "-subj", f"/CN={name}", "-keyout", self.file(f"{name}.key"),
            "-out", self.file(f"{name}.csr"),
        )
        extensions = f"basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage={usage}\n"
        if san:
            extensions += f"subjectAltName={san}\n"
        self.file("leaf.ext").write_text(extensions, encoding="ascii")
        run(
            self.openssl, "x509", "-req", "-in", self.file(f"{name}.csr"),
            "-CA", self.file(f"{role}-ca.crt"), "-CAkey", self.file(f"{role}-ca.key"),
            "-CAcreateserial", "-days", "2", "-extfile", self.file("leaf.ext"),
            "-out", self.file(f"{name}.crt"),
        )

    def tls(self, name, trust):
        return {
            "certificate": f"{name}.crt",
            "private_key": f"{name}.key",
            "ca": f"{trust}-ca.crt",
        }


class ResponseGate(socketserver.ThreadingTCPServer):
    """A TCP relay that can stall worker responses without terminating TLS."""

    daemon_threads = True

    def __init__(self, target):
        super().__init__((LOOPBACK, 0), Relay)
        self.target = target
        self.lock = threading.Lock()
        self.wire_bytes = 0
        self.responses = threading.Event()
        self.responses.set()

    def byte_count(self):
        with self.lock:
            return self.wire_bytes

    def start(self, stack):
        stack.enter_context(self)
        thread = threading.Thread(target=self.serve_forever, kwargs={"poll_interval": 0.05})
        thread.start()
        stack.callback(thread.join)
        stack.callback(self.shutdown)


class Relay(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            with socket.create_connection(self.server.target, timeout=2) as worker:
                worker.settimeout(None)
                control = self.request
                while True:
                    sockets = [control, worker] if self.server.responses.is_set() else [control]
                    readable, _, _ = select.select(sockets, [], [], 0.05)
                    for source in readable:
                        if source is worker and not self.server.responses.is_set():
                            continue
                        data = source.recv(64 * 1024)
                        if not data:
                            return
                        with self.server.lock:
                            self.server.wire_bytes += len(data)
                        (worker if source is control else control).sendall(data)
        except OSError:
            return


def start_etcd(pki, stack):
    prefix = f"osdl-qa-{uuid.uuid4().hex[:10]}"
    names = [f"{prefix}-kv{i}" for i in range(3)]
    network = f"{prefix}-net"
    run("docker", "network", "create", network)
    stack.callback(run, "docker", "network", "rm", network)
    cluster = ",".join(f"{name}=https://{name}:2380" for name in names)
    endpoints = []
    for name in names:
        pki.issue(name, "etcd", "serverAuth,clientAuth", f"IP:{LOOPBACK},DNS:{name}")
        run(
            "docker", "run", "--detach", "--name", name, "--network", network,
            "-p", f"{LOOPBACK}::2379",
            "--mount", f"type=bind,source={pki.root},target=/pki,readonly",
            IMAGE, "/usr/local/bin/etcd",
            f"--name={name}", "--data-dir=/tmp/etcd",
            "--listen-client-urls=https://0.0.0.0:2379",
            f"--advertise-client-urls=https://{name}:2379",
            "--listen-peer-urls=https://0.0.0.0:2380",
            f"--initial-advertise-peer-urls=https://{name}:2380",
            f"--initial-cluster={cluster}", f"--initial-cluster-token={prefix}",
            "--client-cert-auth", "--trusted-ca-file=/pki/etcd-ca.crt",
            f"--cert-file=/pki/{name}.crt", f"--key-file=/pki/{name}.key",
            "--peer-client-cert-auth", "--peer-trusted-ca-file=/pki/etcd-ca.crt",
            f"--peer-cert-file=/pki/{name}.crt", f"--peer-key-file=/pki/{name}.key",
        )
        stack.callback(run, "docker", "rm", "--force", name)
        endpoints.append(f"https://{run('docker', 'port', name, '2379/tcp')}")
    return names, endpoints


def terminate(process):
    if process.poll() is None:
        process.kill()
        process.wait(timeout=10)


def cli(binary, config, *args):
    return run(binary, "--cluster-config", config, *args, env=LABADM_ENV)


def start(stack, binary, config, *args, env=LABADM_ENV):
    process = subprocess.Popen(
        [binary, "--cluster-config", config, *args],
        stdout=subprocess.DEVNULL, env=env,
    )
    stack.callback(terminate, process)
    return process


def rejected(binary, config, *args):
    failure = subprocess.run(
        [binary, "--cluster-config", config, *args], capture_output=True, text=True, timeout=30, env=LABADM_ENV,
    )
    assert failure.returncode != 0, failure.stdout
    return failure.stderr


def write_config(root, name, **data):
    path = root / f"{name}.json"
    # JSON is a subset of the CLI's YAML format.
    path.write_text(json.dumps(data), encoding="utf-8")
    return path


def command_count(root, worker, command):
    with contextlib.closing(sqlite3.connect(root / worker / f"{worker}.db")) as db:
        return db.execute("SELECT count(*) FROM command_log WHERE command_id = ?", (command,)).fetchone()[0]


def initialize(pki, binary, controls, endpoints):
    pki.issue("etcd-client", "etcd", "clientAuth")
    pki.issue("operator", "operator", "clientAuth")
    pki.issue("control-client", "control", "clientAuth")
    control_configs = []
    for i, port in enumerate(controls):
        name = f"controller-{i}"
        pki.issue(name, "control", "serverAuth", f"IP:{LOOPBACK}")
        control_configs.append(write_config(
            pki.root, name, cluster="lab", listen=f"{LOOPBACK}:{port}",
            incoming=pki.tls(name, "operator"), workers=pki.tls("control-client", "worker"),
            directory={"endpoints": endpoints, "tls": pki.tls("etcd-client", "etcd")},
        ))
    wait_for(lambda: cli(binary, control_configs[0], "init"))
    cli(binary, control_configs[1], "init")
    return control_configs


def gateway_fixture(stack, pki, controllers, gateway_ports):
    root = pki.root
    client_env = LABADM_ENV.copy()
    if sys.platform == "linux":
        client_env.update(
            XDG_CONFIG_HOME=str(root / "gateway-config"),
            XDG_CACHE_HOME=str(root / "gateway-cache"),
            XDG_STATE_HOME=str(root / "gateway-state"),
            XDG_RUNTIME_DIR=str(root / "gateway-runtime"),
        )
        Path(client_env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        instances = Path(client_env["XDG_CONFIG_HOME"]) / "osdl" / "instances"
    elif sys.platform == "win32":
        # ProjectDirs uses Windows Known Folders. An isolated, unique binding
        # directory leaves existing laboratory instances untouched.
        instances = Path(os.environ["APPDATA"]) / "scienceol" / "osdl" / "config" / "instances"
    else:
        return None
    instances.mkdir(parents=True, exist_ok=True)
    instance_dir = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="gateway-qa-", dir=instances)))

    def b64(data):
        return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")

    key = pki.file("account-signing.key")
    run(pki.openssl, "genpkey", "-algorithm", "ED25519", "-out", key)
    spki = subprocess.check_output([pki.openssl, "pkey", "-in", key, "-pubout", "-outform", "DER"], timeout=10)
    assert spki[:12] == bytes.fromhex("302a300506032b6570032100") and len(spki) == 44
    binding = {
        "issuer": "https://liyanlabs.local", "channel": "dev", "account_id": "cluster-fixture",
        "lab_id": str(uuid.uuid4()), "instance": instance_dir.name, "key_id": "fixture",
        "public_key": b64(spki[-32:]),
    }
    write_config(instance_dir, "account", **binding)

    def token(scope):
        now = int(time.time())
        claims = {
            "iss": binding["issuer"], "sub": binding["account_id"], "aud": binding["lab_id"],
            "channel": binding["channel"], "device_id": str(uuid.uuid4()), "device_generation": 1,
            "installation_id": str(uuid.uuid4()), "iat": now, "exp": now + 300, "scope": scope,
        }
        header = {"alg": "EdDSA", "typ": "liyanlabs.opensdl.identity+jwt", "kid": binding["key_id"]}
        signing_input = f"{b64(json.dumps(header).encode())}.{b64(json.dumps(claims).encode())}"
        message = pki.file("account-claims.txt")
        message.write_text(signing_input, encoding="ascii")
        signature = subprocess.check_output(
            [pki.openssl, "pkeyutl", "-sign", "-rawin", "-inkey", key, "-in", message], timeout=10,
        )
        return f"{signing_input}.{b64(signature)}"

    for path, port in zip(controllers, gateway_ports):
        config = json.loads(path.read_text())
        config["account_gateway"] = {"listen": f"{LOOPBACK}:{port}", "instance": binding["instance"]}
        path.write_text(json.dumps(config), encoding="utf-8")
    return client_env, token("opensdl:read"), token("opensdl:read opensdl:operate")


def check_gateway_inventory_and_dispatch(binary, root, port, fixture, home, peer):
    env, read_token, operate_token = fixture
    lab = binary.with_name("lab" + binary.suffix)
    endpoint = f"http://{LOOPBACK}:{port}"
    read_env = {**env, "OSDL_AUTH_TOKEN": read_token}
    operate_env = {**env, "OSDL_AUTH_TOKEN": operate_token}
    arg = (lab, "--endpoint", endpoint)
    home_id = f"{home}:{DEVICE}"
    peer_id = f"{peer}:{DEVICE}"
    listing = wait_for(lambda: run(*arg, "device", "list", "--json", env=read_env))
    anonymous = subprocess.run((*arg, "device", "list"), env=env, capture_output=True, timeout=10)
    assert anonymous.returncode != 0
    assert {home_id, peer_id} <= {device["id"] for device in json.loads(listing)}
    device = json.loads(run(*arg, "device", "get", home_id, "--json", env=read_env))
    assert device["id"] == home_id and not device["transport_id"]
    waited = json.loads(run(*arg, "device", "wait", f"id:{home_id}", "--timeout", "3s", "--json", env=read_env))
    assert waited["id"] == home_id

    blocked = subprocess.run(
        (*arg, "send", peer_id, "start", "--command-id", "gateway-read-only"),
        env=read_env, capture_output=True, timeout=10,
    )
    assert blocked.returncode != 0
    assert command_count(root, peer, "gateway-read-only") == 0
    result = json.loads(run(*arg, "send", peer_id, "start", "--command-id", "gateway-routed",
                            "--json", env=operate_env))
    assert result["device_id"] == peer_id and result["command_id"] == "gateway-routed"
    assert command_count(root, home, "gateway-routed") == 0
    assert command_count(root, peer, "gateway-routed") == 1


def check_gateway_event_capacity(binary, port, fixture, worker):
    env, read_token, operate_token = fixture
    lab = binary.with_name("lab" + binary.suffix)
    endpoint = f"http://{LOOPBACK}:{port}"
    events_cmd = (lab, "--endpoint", endpoint, "events", "--json", "--kinds", "command_result")
    reader_env = {**env, "OSDL_AUTH_TOKEN": read_token}
    with contextlib.ExitStack() as cleanup, concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        streams = []
        try:
            for _ in range(8):
                process = subprocess.Popen(events_cmd, stdout=subprocess.PIPE, text=True, env=reader_env)
                cleanup.callback(process.stdout.close)
                cleanup.callback(terminate, process)
                streams.append(process)
            first_events = [executor.submit(process.stdout.readline) for process in streams]
            end = time.monotonic() + 8
            while time.monotonic() < end and not all(event.done() for event in first_events):
                run(lab, "--endpoint", endpoint, "send", f"{worker}:{DEVICE}", "start",
                    env={**env, "OSDL_AUTH_TOKEN": operate_token})
                time.sleep(0.1)
            for event in first_events:
                assert json.loads(event.result(timeout=2))["kind"] == "command_result"
            denied = subprocess.run(events_cmd, capture_output=True, text=True, env=reader_env, timeout=5)
            assert denied.returncode != 0 and "cluster event subscriptions are busy" in denied.stderr
        finally:
            for process in streams:
                terminate(process)


def check_gateway_worker_loss(binary, port, fixture, home, peer, stopped_worker, rejoin):
    env, read_token, operate_token = fixture
    lab = binary.with_name("lab" + binary.suffix)
    endpoint = f"http://{LOOPBACK}:{port}"
    read_env = {**env, "OSDL_AUTH_TOKEN": read_token}
    operate_env = {**env, "OSDL_AUTH_TOKEN": operate_token}
    arg = (lab, "--endpoint", endpoint)
    events = subprocess.Popen(
        (*arg, "events", "--json", "--kinds", "command_result,worker_unavailable"),
        stdout=subprocess.PIPE, text=True, env=read_env,
    )

    def await_event(kind, field, expected):
        for line in events.stdout:
            event = json.loads(line)
            if event["kind"] == kind and event["payload"].get(field) == expected:
                return event
        raise AssertionError(f"gateway event stream closed before {kind}")

    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
        try:
            primed = executor.submit(await_event, "command_result", "device_id", f"{home}:{DEVICE}")
            for attempt in range(10):
                run(*arg, "send", f"{home}:{DEVICE}", "start",
                    "--command-id", f"gateway-prime-{attempt}", env=operate_env)
                if primed.done():
                    break
                time.sleep(0.1)
            primed.result(timeout=3)
            down = executor.submit(await_event, "worker_unavailable", "worker", home)
            terminate(stopped_worker)
            down.result(timeout=5)

            # The same subscriber still receives events for another worker.
            retained = executor.submit(await_event, "command_result", "command_id", "gateway-survivor")
            run(*arg, "send", f"{peer}:{DEVICE}", "start", "--command-id", "gateway-survivor",
                env=operate_env)
            assert retained.result(timeout=5)["payload"]["device_id"] == f"{peer}:{DEVICE}"

            recovered = executor.submit(await_event, "command_result", "device_id", f"{home}:{DEVICE}")
            restored = rejoin()
            try:
                end = time.monotonic() + 8
                while time.monotonic() < end and not recovered.done():
                    run(*arg, "send", f"{home}:{DEVICE}", "start", env=operate_env)
                    time.sleep(0.1)
                recovered.result(timeout=2)
            finally:
                terminate(restored)
        finally:
            terminate(events)
            events.stdout.close()

    listing = run(*arg, "device", "list", "--json", env=read_env)
    available = {device["id"] for device in json.loads(listing)}
    assert f"{peer}:{DEVICE}" in available and f"{home}:{DEVICE}" not in available
    print("account gateway verified: routed IDs, exact dispatch, scopes and worker-loss isolation", flush=True)


def assert_local_inventory(binary, config, workers):
    listing = cli(binary, config, "devices")
    for worker in workers:
        assert f"{worker}\t{DEVICE}\t" in listing, listing
    return len(listing.encode())


def etcd_metrics(pki, endpoints):
    tls = ssl.create_default_context(cafile=str(pki.file("etcd-ca.crt")))
    tls.load_cert_chain(str(pki.file("etcd-client.crt")), str(pki.file("etcd-client.key")))
    counts = 0
    for endpoint in endpoints:
        with urllib.request.urlopen(f"{endpoint}/metrics", context=tls, timeout=4) as response:
            metrics = response.read().decode()
        for line in metrics.splitlines():
            if line.startswith("grpc_server_handled_total{") and all(
                label in line for label in ('grpc_service="etcdserverpb.KV"', 'grpc_method="Range"', 'grpc_code="OK"')
            ):
                counts += int(float(line.split()[-1]))
    return counts


def measure(binary, config, pki, endpoints, gate, expected_workers, history):
    sample_worker = expected_workers[1]
    cli(binary, config, "device", sample_worker, DEVICE)
    def lookup(_):
        start_time = time.perf_counter()
        payload = cli(binary, config, "device", sample_worker, DEVICE)
        return 1000 * (time.perf_counter() - start_time), len(payload.encode())

    before = etcd_metrics(pki, endpoints)
    wire_before = gate.byte_count()
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as executor:
        samples = list(executor.map(lookup, range(24)))
    wire_per_lookup = (gate.byte_count() - wire_before) / len(samples)
    db_reads = etcd_metrics(pki, endpoints) - before
    assert db_reads == 48, db_reads

    before = etcd_metrics(pki, endpoints)
    start_time = time.perf_counter()
    bytes_out = assert_local_inventory(binary, config, expected_workers)
    inventory_time = 1000 * (time.perf_counter() - start_time)
    inventory_reads = etcd_metrics(pki, endpoints) - before
    assert inventory_reads == 1 + (len(expected_workers) + 7) // 8, inventory_reads
    times = sorted(sample[0] for sample in samples)
    print(
        f"{len(expected_workers)} workers, {history} seeded events: cold device lookup p50={statistics.median(times):.1f} ms, "
        f"p95={times[22]:.1f} ms, {samples[0][1]} stdout bytes, "
        f"{wire_per_lookup:.0f} control-to-worker TLS bytes (both directions), "
        f"{db_reads / len(samples):.0f} directory reads per CLI lookup; "
        f"inventory={inventory_time:.1f} ms, {bytes_out} stdout bytes, {inventory_reads} directory reads",
        flush=True,
    )


def exercise(binary, openssl):
    with tempfile.TemporaryDirectory(prefix="osdl-cluster-") as tmp, contextlib.ExitStack() as stack:
        root = Path(tmp).resolve()
        pki = Pki(root, openssl)
        names, endpoints = start_etcd(pki, stack)
        first, second, first_gateway, second_gateway, *worker_ports = ports(14)
        client_paths = []
        for endpoint in endpoints[:2]:
            port = int(endpoint.rsplit(":", 1)[1])
            path = ResponseGate((LOOPBACK, port))
            path.start(stack)
            client_paths.append(path)
        routed_endpoints = [f"https://{LOOPBACK}:{path.server_address[1]}" for path in client_paths]
        controls = initialize(pki, binary, (first, second), routed_endpoints + endpoints[2:])
        fixture = gateway_fixture(stack, pki, controls, (first_gateway, second_gateway))
        control_env = fixture[0] if fixture else LABADM_ENV
        daemons = [start(stack, binary, path, "serve", env=control_env) for path in controls]
        control_endpoints = [f"https://{LOOPBACK}:{port}" for port in (first, second)]
        config = write_config(root, "client", cluster="lab", endpoints=control_endpoints,
                              tls=pki.tls("operator", "control"))
        standby = write_config(root, "standby", cluster="lab", endpoints=control_endpoints[1:],
                               tls=pki.tls("operator", "control"))
        wrong_role = write_config(root, "wrong-role", cluster="lab", endpoints=control_endpoints,
                                  tls=pki.tls("control-client", "control"))
        wait_for(lambda: cli(binary, config, "status"))
        wait_for(lambda: cli(binary, standby, "status"))
        rejected(binary, wrong_role, "status")

        gate = ResponseGate((LOOPBACK, worker_ports[1]))
        gate.start(stack)
        workers = {}
        for index, port in enumerate(worker_ports):
            name = f"worker-{index:02}"
            pki.issue(name, "worker", "serverAuth", f"DNS:{name}.lab.worker.opensdl.internal")
            worker = write_config(
                root, name, cluster="lab", worker=name, listen=f"{LOOPBACK}:{port}",
                incoming=pki.tls(name, "control"),
            )
            workers[name] = start(stack, binary, worker, "join", "--simulation", "--data-dir", root / name)
            route_port = gate.server_address[1] if index == 1 else port
            wait_for(lambda: cli(binary, controls[0], "admit", name, f"{LOOPBACK}:{route_port}"))
            if index == 1:
                measure(binary, config, pki, endpoints, gate, list(workers), 0)

        names_in_order = sorted(workers)
        assert cli(binary, standby, "workers").splitlines() == names_in_order
        home, peer = names_in_order[:2]
        if fixture:
            before = etcd_metrics(pki, endpoints)
            check_gateway_inventory_and_dispatch(binary, root, first_gateway, fixture, home, peer)
            # One membership range, one GetDevice route, one WaitForDevice
            # route and one SendCommand route. Denied requests never read etcd.
            assert etcd_metrics(pki, endpoints) - before == 4
            check_gateway_event_capacity(binary, first_gateway, fixture, home)
        with contextlib.closing(sqlite3.connect(root / peer / f"{peer}.db")) as db:
            db.execute("""
                WITH RECURSIVE sequence(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM sequence WHERE i < 100000)
                INSERT INTO event_log(timestamp, event_type, payload)
                SELECT 1700000000000 + i, 'device_status', '{"temperature":22,"running":false}' FROM sequence
            """)
            db.commit()
        measure(binary, config, pki, endpoints, gate, names_in_order, 100_000)
        rejected(binary, controls[0], "admit", home, f"{LOOPBACK}:{worker_ports[1]}")
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            list(executor.map(lambda _: cli(binary, controls[1], "admit", home,
                                           f"{LOOPBACK}:{worker_ports[0]}"), range(3)))

        def start_heater(worker, command):
            return cli(binary, config, "send", worker, DEVICE, "start", "--command-id", command)

        assert "SUCCEEDED" in start_heater(home, "before-failover")
        assert command_count(root, home, "before-failover") == 1
        assert command_count(root, peer, "before-failover") == 0

        # Hold the response after a request reached an already open worker connection.
        cli(binary, config, "device", peer, DEVICE)
        gate.responses.clear()
        try:
            error = rejected(binary, config, "send", peer, DEVICE, "start", "--command-id", "ambiguous")
            assert "outcome" in error, error
            assert command_count(root, peer, "ambiguous") == 1
            assert command_count(root, home, "ambiguous") == 0
        finally:
            gate.responses.set()

        terminate(daemons[0])
        started = time.perf_counter()
        assert control_endpoints[1] in cli(binary, config, "status")
        print(f"control process failure to standby ready: {(time.perf_counter() - started) * 1000:.1f} ms",
              flush=True)
        assert "SUCCEEDED" in start_heater(peer, "after-failover")
        assert command_count(root, peer, "after-failover") == 1

        events = subprocess.Popen(
            [binary, "--cluster-config", config, "events", peer, "--kind", "command_result"],
            stdout=subprocess.PIPE, text=True, env=LABADM_ENV,
        )
        stack.callback(terminate, events)
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            line = executor.submit(events.stdout.readline)
            try:
                # Keep exercising the actual path until the stream subscription is live.
                for _ in range(20):
                    start_heater(peer, "stream-proof")
                    if line.done():
                        break
                    time.sleep(0.05)
                assert "stream-proof" in line.result(timeout=2)
            finally:
                terminate(events)
                events.stdout.close()

        if fixture:
            def rejoin():
                cli(binary, controls[1], "forget", home)
                before = etcd_metrics(pki, endpoints)
                # Observe the disconnected feed query the missing route
                # before restoring it, so a terminal NotFound loses the test.
                wait_for(lambda: etcd_metrics(pki, endpoints) > before, timeout=8)
                process = start(stack, binary, root / f"{home}.json", "join", "--simulation",
                                "--data-dir", root / home)
                wait_for(lambda: cli(binary, controls[1], "admit", home, f"{LOOPBACK}:{worker_ports[0]}"))
                return process

            check_gateway_worker_loss(binary, second_gateway, fixture, home, peer, workers[home], rejoin)
        else:
            terminate(workers[home])
        assert f"{home}\tunavailable\t" in cli(binary, config, "devices")
        rejected(binary, config, "send", home, DEVICE, "start", "--command-id", "offline")
        assert command_count(root, peer, "offline") == 0

        # The first two client paths can stall while the peer network retains quorum.
        for path in client_paths:
            path.responses.clear()
        try:
            started = time.perf_counter()
            assert control_endpoints[1] in cli(binary, standby, "status")
            print(f"directory recovery with two stalled client paths: {(time.perf_counter() - started) * 1000:.1f} ms",
                  flush=True)
        finally:
            for path in client_paths:
                path.responses.set()

        # The shared directory remains writable with one stopped member.
        run("docker", "stop", "-t", "0", names[0])
        wait_for(lambda: cli(binary, config, "device", peer, DEVICE))
        cli(binary, controls[1], "forget", home)
        assert home not in cli(binary, config, "workers").splitlines()

        # Two members down lose quorum. Neither control may route from cache.
        run("docker", "stop", "-t", "0", names[1])
        rejected(binary, config, "status")
        rejected(binary, config, "send", peer, DEVICE, "start", "--command-id", "no-quorum")
        assert command_count(root, peer, "no-quorum") == 0
        print("cluster routing, certificate roles, inventory, event streaming, non-replay and failover verified")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--labadm", type=Path, default=Path("target/debug/labadm" + (".exe" if os.name == "nt" else "")))
    parser.add_argument("--openssl", default=shutil.which("openssl") or "openssl")
    options = parser.parse_args()
    exercise(options.labadm.resolve(), options.openssl)


if __name__ == "__main__":
    main()
