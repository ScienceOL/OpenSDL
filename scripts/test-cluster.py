#!/usr/bin/env python3
"""Exercise labadm with real etcd quorum, TLS, and simulated OpenSDL workers."""

import argparse
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


def start(stack, binary, config, *args):
    process = subprocess.Popen(
        [binary, "--cluster-config", config, *args],
        stdout=subprocess.DEVNULL, env=LABADM_ENV,
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


def assert_local_inventory(binary, config, workers):
    listing = cli(binary, config, "devices")
    for worker in workers:
        assert f"{worker}\t{DEVICE}\t" in listing, listing
    return len(listing.encode())


def check_shared_worker(stack, pki, binary, lab, operator, admin):
    worker = "shared-worker"
    instance = "shared-lab"
    worker_port, direct_port = ports(2)
    pki.issue(worker, "worker", "serverAuth", f"DNS:{worker}.lab.worker.opensdl.internal")
    worker_config = write_config(
        pki.root, worker, cluster="lab", worker=worker, listen=f"{LOOPBACK}:{worker_port}",
        incoming=pki.tls(worker, "control"),
    )
    endpoint = f"http://{LOOPBACK}:{direct_port}"
    env = {**LABADM_ENV, "OSDL_AUTH_TOKEN": "fixture-" + uuid.uuid4().hex}
    data = pki.root / worker
    process = subprocess.Popen(
        [lab, "serve", "--cluster-worker", worker_config, "--instance", instance,
         "--listen", f"{LOOPBACK}:{direct_port}", "--socket=disabled",
         "--auth", "static", "--simulation", "--data-dir", data],
        stdout=subprocess.DEVNULL, env=env,
    )
    stack.callback(terminate, process)

    def read():
        return json.loads(run(lab, "--endpoint", endpoint, "device", "get", DEVICE, "--json", env=env))

    wait_for(read)
    wait_for(lambda: cli(binary, admin, "admit", worker, f"{LOOPBACK}:{worker_port}"))
    result = cli(binary, operator, "send", worker, DEVICE, "start", "--command-id", "shared-engine")
    assert "SUCCEEDED" in result, result
    wait_for(lambda: read()["properties"]["running"] is True)
    with contextlib.closing(sqlite3.connect(data / f"{instance}.db")) as db:
        count = db.execute(
            "SELECT count(*) FROM command_log WHERE command_id = ?", ("shared-engine",),
        ).fetchone()[0]
    assert count == 1

    unauthenticated = subprocess.run(
        [lab, "--endpoint", endpoint, "device", "get", DEVICE],
        capture_output=True, text=True, timeout=8, env=LABADM_ENV,
    )
    assert unauthenticated.returncode != 0 and "missing authorization header" in unauthenticated.stderr, unauthenticated
    cli(binary, admin, "forget", worker)
    terminate(process)
    print("direct lab and mTLS worker share live engine state and one command log", flush=True)


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


def exercise(binary, lab, openssl):
    with tempfile.TemporaryDirectory(prefix="osdl-cluster-") as tmp, contextlib.ExitStack() as stack:
        root = Path(tmp).resolve()
        pki = Pki(root, openssl)
        names, endpoints = start_etcd(pki, stack)
        first, second, *worker_ports = ports(12)
        client_paths = []
        for endpoint in endpoints[:2]:
            port = int(endpoint.rsplit(":", 1)[1])
            path = ResponseGate((LOOPBACK, port))
            path.start(stack)
            client_paths.append(path)
        routed_endpoints = [f"https://{LOOPBACK}:{path.server_address[1]}" for path in client_paths]
        controls = initialize(pki, binary, (first, second), routed_endpoints + endpoints[2:])
        daemons = [start(stack, binary, path, "serve") for path in controls]
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
        check_shared_worker(stack, pki, binary, lab, config, controls[0])

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
    parser.add_argument("--lab", type=Path, help="Direct lab binary adjacent to labadm by default")
    parser.add_argument("--openssl", default=shutil.which("openssl") or "openssl")
    options = parser.parse_args()
    lab = options.lab or options.labadm.with_name("lab" + options.labadm.suffix)
    exercise(options.labadm.resolve(), lab.resolve(), options.openssl)


if __name__ == "__main__":
    main()
