# Account authentication

OpenSDL 0.5.2 can bind one laboratory instance to a LiyanLabs account:

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
`--auth static` retains explicit static bearer support for standalone/supervised
services. An unauthenticated static listener is restricted to loopback. Unix
sockets keep their filesystem administrator authority, including Shutdown.

The separate `osdl.v1.Authentication.Info` RPC exposes only mode, lab UUID,
instance, issuer and channel. Mode is `account`, `bearer`, or `local`. All TCP
operational account requests carry `authorization: Bearer <assertion>`. OpenSDL
verifies the pinned Ed25519 key, exact issuer/account/lab/channel, device UUID,
positive generation, installation UUID, issuance/expiry and a maximum 300-second
lifetime locally. Scope is `opensdl:read` or `opensdl:read opensdl:operate`;
SendCommand requires operate and account assertions never authorize Shutdown.

`Authentication.Renew` verifies the newly presented assertion and returns
`expires_at` (Unix seconds). It extends active leases for precisely the same device,
generation and installation. The Runner renews before swapping its in-memory
bearer. Existing event streams and camera publisher processes remain in place.
Expired leases cannot be revived: the stream ends with UNAUTHENTICATED and the
local publisher is dropped. Hardware stop/replay commands are never sent by
lease expiry. Only active streams own leases; closing the final stream removes
its registry entry. New operations always verify their own presented assertion.

Public-key pairing is not implemented by this authentication mode.

Authentication failures use gRPC UNAUTHENTICATED and metadata
`opensdl-auth-error`: `account_mismatch`, `authorization_expired`, or
`invalid_assertion`. Account mismatch and expiry are classified only after
signature and claim-shape verification. Active stream expiry carries the same
`authorization_expired` metadata. No expected owner ID or assertion bytes are
returned. Scope violations and account Shutdown remain PERMISSION_DENIED.
