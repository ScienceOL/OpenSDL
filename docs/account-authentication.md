# Account authentication

An account-authenticated OpenSDL laboratory has one bound LiyanLabs owner
and can admit other accounts from the same environment.

```sh
lab login --server https://liyanlabs.com --instance laboratory
lab serve --instance laboratory --auth account --listen 100.64.0.9:50051
```

Open the printed verification URL, sign in, and explicitly approve the instance.
The CLI polls at the server interval and stores only the public binding in the
platform configuration directory, under `instances/laboratory/account.json`.
There is no saved account access token, refresh token, or password. Publication
is atomic and create-only. Repeating login for a bound instance fails; reuse its
binding to retain its laboratory identity. Moving/replacing a binding is an
explicit administrator operation and changes which account/realm the service trusts.

Supported exact origins are `https://liyanlabs.local` (dev),
`https://test.liyanlabs.com` (beta), `https://uat.liyanlabs.com` (rc), and
`https://liyanlabs.com` (stable). Aliases and trailing slashes are rejected.
HTTPS uses certificate validation and system trust roots, including a locally
installed Dev CA. Account TCP listeners accept only loopback or an explicit
Tailscale IPv4 address. The tailnet provides network encryption and access control.

`--auth account` rejects `--auth-token` and `OSDL_AUTH_TOKEN`. The default
`--auth local` uses protected local authority. Explicit `--auth static` requires
a bearer for standalone/supervised services. Unix sockets retain administrator
authority; Windows uses a distinct owner-authenticated loopback admin listener.
See [paired and local authentication](paired-authentication.md).

The separate `osdl.v1.Authentication.Info` RPC exposes only mode, lab UUID,
instance, issuer and channel. Mode is `account`, `public_key`, `bearer`, or `local`. All TCP
operational account requests carry `authorization: Bearer <assertion>`. OpenSDL
verifies the pinned Ed25519 key, exact issuer/lab/channel, signed account
subject, device UUID, positive generation, installation UUID, issuance/expiry
and a maximum 300-second lifetime before consulting the lab's role assignment.
The bound owner has implicit membership. Other valid accounts need an explicit grant.

## Sharing a laboratory

Run these commands on the host as the OS user running the laboratory service.
Use the exact LiyanLabs account ID from the laboratory's environment.

```sh
lab auth account list --instance laboratory
lab auth account grant ACCOUNT_ID --role viewer --instance laboratory
lab auth account grant ACCOUNT_ID --role operator --instance laboratory
lab auth account revoke ACCOUNT_ID --instance laboratory
```

| Role | Capabilities |
| --- | --- |
| Viewer | Read status, devices and observations, including event and camera streams |
| Operator | Viewer capabilities and device commands with operate scope |
| Admin | Operator capabilities and laboratory profile edits with operate scope |
| Bound owner | Admin capabilities |

A read-only assertion limits every role to observation. The laboratory advertises
profile editability through authenticated Status. Profile updates require the
current revision. Service shutdown and membership administration require the
protected local OS administrator endpoint. The paired-computer grant commands
remain under `lab auth invite`, `lab auth list` and `lab auth revoke`.

The membership database lives in a private subdirectory alongside the instance's
`account.json`. The running service is the sole writer. Grant and revoke take
effect live. Revocation and role changes release the affected account's active
subscriptions and camera publishers before confirmation. An operation accepted
before a revoke may already have reached the instrument. The service audit log
target `opensdl::account_audit` records the verified subject, device, generation
and installation for device command submissions and profile updates.

`Authentication.Renew` verifies the newly presented assertion and returns
`expires_at` (Unix seconds). It extends active leases for precisely the same account subject, device,
generation and installation. The Runner renews before swapping its in-memory
bearer. Existing event streams and camera publisher processes remain in place.
Expired leases cannot be revived: the stream ends with UNAUTHENTICATED and the
local publisher is dropped. Hardware stop/replay commands are never sent by
lease expiry. Only active streams own leases; closing the final stream removes
its registry entry. One lifecycle task per authorized stream owns its upstream
and watches expiry and consumer cancellation independently of HTTP/2 flow control.
A one-item handoff bounds buffering; queued data is discarded after expiry.
Static streams keep their direct path. New operations always verify their own
presented assertion.

Independent public-key pairing is a separate explicit mode, described in
[paired authentication](paired-authentication.md).

Authentication failures use gRPC UNAUTHENTICATED and metadata
`opensdl-auth-error`: `account_mismatch`, `authorization_expired`, or
`invalid_assertion`. `account_mismatch` indicates that a valid account lacks laboratory membership or its
active lease was revoked. Expired requests and streams carry
`authorization_expired`. Authentication errors omit the owner ID and assertion bytes. Scope violations and account Shutdown remain PERMISSION_DENIED.
