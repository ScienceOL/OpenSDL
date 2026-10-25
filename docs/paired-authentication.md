# Paired-computer and local-only laboratories

`lab serve` selects one explicit authority. The default is `--auth local`.
Account HTTP and public-key HTTPS never share one listener. The independent
public-key mode works without Casdoor or a LiyanLabs account.

Instance names contain 1–128 ASCII letters, digits, `-` or `_`. The same name
is used for account bindings, private discovery and lockfiles; names are never
silently rewritten into another name.

On Unix, a custom `--socket` requires a canonical parent directory owned by the
service user with mode `0700`; the socket itself is `0600`. Default runtime
directories are created privately. Existing systemd services with a custom
runtime directory should set both `User=` and `RuntimeDirectoryMode=0700`.
Alternatively create a dedicated directory as the service user with
`install -d -m 700 /path/to/private-run` and put the socket there. Do not use a
shared `0755` parent or change another service's directory permissions. A
private leaf that is a symlink is rejected; resolve system parent aliases
before specifying an explicit path.

## Independent laboratory

```sh
lab serve --auth public-key --channel stable --instance laboratory \
  --listen 100.64.0.9:50051 --data-dir /path/to/laboratory-data
lab auth invite --instance laboratory
```

The local invitation wizard asks for Read or Operate and a grant duration.
For automation, both choices are explicit:

```sh
lab auth invite --instance laboratory --access read --valid-for 30d
lab auth invite --instance laboratory --access operate --valid-for until-revoked
lab auth list --instance laboratory
lab auth revoke --instance laboratory GRANT_UUID
```

`--valid-for` accepts positive `NUMBERd`, `NUMBERh`, or `until-revoked` (finite
durations up to 20 years). Each invitation lasts 600 seconds independently of
the grant deadline. Until-revoked grants have a visible 20-year certificate
lifetime; expiry requires a new explicit pairing, with no automatic renewal.
The copyable invitation is a full `opensdl-pair-v1.` envelope, at most 8 KiB,
containing a one-use capability and the pinned laboratory CA. Keep it in the
administrator's authenticated local/SSH terminal. Do not put it in chat,
URLs, command arguments, public configuration, logs, or cloud requests.

On the client:

```sh
lab pair --name laboratory
lab --connection laboratory status
lab --connection laboratory device list
lab --connection laboratory events
```

Pair reads hidden terminal input, verifies TLS and the exact laboratory,
instance and channel, displays server-confirmed scope/deadline, then asks for
confirmation. Controlled automation can pipe the invitation through stdin
with `--stdin --yes`; the secret still never goes in argv. `--label` is display
text; names do not confer authority. `--replace` explicitly replaces a local
profile, without remotely revoking the previous grant.

A client key and exact signed CSR are durably saved before Pair. Retrying the
same invitation and CSR within its 600-second lifetime returns the same grant
and certificate. Another CSR cannot redeem it. If the response is lost, retry
the same named profile and invitation; do not generate a new key. After expiry,
ask the administrator to inspect/revoke the ambiguous grant and issue a new
invitation. Only a finalized profile can open an operational mTLS connection.

Read grants cannot send commands. Ordinary grants cannot shut down the service
or administer grants. Revoke blocks new calls and releases active stream and
publisher ownership before acknowledgment; already accepted hardware commands
are not an emergency stop.

## Local-only service

```sh
lab serve --auth local --instance private
lab --instance private status
lab --instance private stop
```

Unix uses an owner-private directory and 0600 UDS. An explicitly requested TCP
listener must be loopback and uses an automatically generated capability.
Windows uses ephemeral loopback plus the same automatic capability in private
discovery storage. Public lockfiles and Status contain no capability. Same-user
CLI discovery reads the protected record without asking the user for a token.
Local mode rejects LAN, Tailnet and all-interface bindings and disables mDNS
advertisement. Account/public-key services retain separate local administrator
authority: a Unix UDS or a distinct authenticated Windows loopback listener.

Supervisors intentionally supplying `OSDL_AUTH_TOKEN` must select `--auth static`.
Static mode requires a nonempty token; it is an explicit internal/standalone
mode, never a fallback from local/account/public-key authority.

## Persistence and client integration

Laboratory CA/server identity, invitations, grants and revoke state live under
`<data-dir>/<instance>-public-key`. Only the running service writes the auth
SQLite database. Missing or corrupt established identity/database is a recovery
error; restore the consistent local directory rather than silently regenerating
an identity. Do not edit the database or a key list behind the running service.

`osdl-auth` is the shared engine-free client crate. `Invitation::parse` enforces
the closed envelope and normalized Tailnet HTTPS endpoint. Call
`inspect_invitation` before private confirmation, then `ProfileStore::pair`;
`ProfileStore::connect` returns the pinned operational Tonic channel.
`prepare_pair` exposes the durable pending CSR for an owning client workflow.
`ProfileStore::metadata(name)` returns typed public lab/instance/channel/endpoint,
access, grant deadline, and Pending or Paired state (grant ID and certificate
deadline), without exposing CA or private key material. Routing/catalog lookup
and connection deletion remain owned by the caller; the store selects by name.

`ProfileStore::open` takes an absolute caller-owned root with an existing parent.
It provides atomic private records: Windows user DPAPI plus owner-only protected
DACLs, or Unix 0700/0600 ownership and no-symlink checks. Keys are never Debug or
public DTO values. Standalone profiles use `paired-clients` in the platform
state directory. A Runner supplies a separate stable channel/device/local-profile
namespace and checks its current installation/account/generation authority at
use; build versions and release digests must not name the stored key. Desktop
prompts collect invitation and confirmation only, never key persistence IPC.

The wire contracts are `Pairing.DescribeInvitation`, `Pairing.Pair`, and the
local-only `GrantAdministration.Invite`, `ListGrants` (100-row pages), `Revoke`.
Enrollment messages are bounded to 8 KiB, at most 256 live invitations, and
8 concurrent enrollment workers. Secret-bearing wire messages have redacted
Debug implementations. Operational errors use `opensdl-auth-error` metadata:
`client_certificate_required`, `grant_inactive`, `grant_expired`, `grant_revoked`.
An absent/removed grant deliberately shares `grant_inactive` without disclosing
its historical membership to an unauthorized caller.

Per-operation authorization hashes the verified TLS peer leaf and checks the
in-memory grant/scope/deadline. It never queries SQLite or a cloud service.
A deadline index retires expired grants during subsequent enrollment, without
scanning historical grants. Streams retain one bounded handoff item and an
independent lifetime task, so unread HTTP/2 bodies cannot retain publishers.

## Cryptography source evidence

The implementation uses rcgen 0.14.10, repository `rustls/rcgen`, crate VCS commit
`f4a3b165c467c070d5c74b86acc2c3f0a0be10ab`. Its `src/csr.rs` verifies CSR signatures
through x509-parser, then imports requested extensions; the server discards all
requested parameters and constructs non-CA, clientAuth-only P-256 certificates.
The caller also rejects trailing DER bytes, which rcgen's parser otherwise
ignores. `Issuer::from_ca_cert_pem` does not prove key consistency; persisted CA
and server key/certificate consistency is therefore checked at startup.
Tonic 0.14.6's TLS server accepts absent client certificates only through
`client_auth_optional(true)` with a pinned CA, and still verifies any certificate
presented. The ring Rustls provider is explicitly installed. Client trust has
only the invitation CA and `lab-<UUID>.opensdl.invalid` hostname, with no system
roots or certificate/hostname verification bypass.
