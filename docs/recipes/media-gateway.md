# Recipe — ONVIF camera media gateway

Replaces `media_gateway.rs`. The engine spawns `mediamtx` and exposes one
ONVIF camera's RTSP feeds at stable URLs (RTSP / HLS / WebRTC) under the
local gateway. Optional remote-RTMP republishing for off-site viewing.

## Prerequisites

- `mediamtx` available in `$PATH` (or its path baked into the gateway
  config — see `crates/osdl-core/src/media/mediamtx.rs`).
- ONVIF camera reachable on the LAN, with the credentials of an account
  that can read its RTSP streams.

## Walk-through

### 1. Drop credentials into the config

Open [`configs/onvif-camera.yaml`](configs/onvif-camera.yaml) and replace
the `USER:PASS@HOST` placeholders with real values, OR keep a copy
outside the repo. Don't commit secrets.

```sh
cp docs/recipes/configs/onvif-camera.yaml /tmp/cam1.yaml
# edit /tmp/cam1.yaml
```

### 2. Boot the server

```sh
lab serve --detach \
  --instance camgw \
  --config /tmp/cam1.yaml \
  --registry $(pwd)/registry/unilabos
```

The engine validates the media source before spawning mediamtx. If the
camera URL is malformed or the binary can't be found, the server logs the
error and exits without leaving a process behind.

### 3. Discover the published URLs

```sh
lab --instance camgw events --kinds media_source_online,media_gateway_down --json &
```

You'll see one event per media source with the full list of endpoints,
e.g.:

```
{"kind":"media_source_online","payload":{"id":"cam1","description":"ONVIF dev camera",
 "endpoints":[{"protocol":"rtsp","location":"local","url":"rtsp://127.0.0.1:8554/cam1"},
              {"protocol":"hls","location":"local","url":"http://127.0.0.1:8888/cam1/index.m3u8"},
              {"protocol":"webrtc","location":"local","url":"http://127.0.0.1:8889/cam1"}]}}
```

### 4. Verify

```sh
ffprobe -rtsp_transport tcp rtsp://127.0.0.1:8554/cam1
ffplay  -rtsp_transport tcp rtsp://127.0.0.1:8554/cam1_h264
open http://127.0.0.1:8888/cam1_h264          # HLS in browser
```

### 5. Stop

```sh
lab --instance camgw stop
```

The engine signals mediamtx to terminate gracefully on shutdown.

## Notes

- HEVC sources are auto-transcoded to H.264 via an ffmpeg sidecar when
  `h264_transcode: true`. The transcoded output appears as the `_h264`
  suffix path.
- For remote ingest (push to SRS), uncomment the `remote_rtmp:` block.
  See `crates/osdl-core/src/media/onvif_camera.rs` for the validation
  rules. Note: with HEVC passthrough as the default, the mediamtx push
  to SRS is `-c copy` of the HEVC stream — vanilla RTMP carries only
  H.264, so this path relies on SRS's Enhanced RTMP / HEVC-over-RTMP
  support (SRS 5+ has it). If the downstream browser's WHEP stack
  refuses to negotiate HEVC, flip `h264_transcode: true` to force an
  H.264 republish from the camera's main or sub stream.

## Remote ingest (push to SRS)

Each camera can optionally push to a remote SRS instance so viewers
outside the LAN (e.g. a teammate the camera is shared with) can still
see the live feed. mediamtx re-publishes the camera's already-encoded
stream to SRS via `ffmpeg -c copy`, so there is no extra encoding cost
on the lab host.

### Local dev — bring up SRS with `just dev`

A local SRS container is included in the dev stack automatically
(`docker/docker-compose.srs.yaml`). After `just dev`:

- RTMP ingest:  `rtmp://localhost:1935/<app>/<stream>`
- HTTP-FLV:     `http://localhost:18085/<app>/<stream>.flv`
- HLS:          `http://localhost:18085/<app>/<stream>.m3u8`
- WHEP (POST):  `http://localhost:1985/rtc/v1/whep/?app=<app>&stream=<stream>`
- WebRTC media: UDP `localhost:8000`

**Two HTTP ports** — SRS splits its HTTP surface: HLS/FLV on the
`http_server` port (18085) and the WHEP signalling POST on the `http_api`
port (1985). The browser's WebRTC media flows over UDP 8000.

**UDP 8000 is not remapped** — SRS bakes the internal UDP port
(`a=candidate:... 127.0.0.1 8000`) into its SDP answer, so Docker's
host-side port remapping is invisible to the browser. SRS_RTC_PORT
must stay `8000`. UDP 8000 is free on the dev host (TCP 8000 is taken
by MinIO; TCP and UDP are independent sockets).

(HTTP-server port defaults to 18085, not 8080, because the dev stack's
OpenFGA `network-service` already occupies 8080. See
`docker/docker-compose.srs.yaml`.)

If any of the dev host ports 1935, 18085, or 1985 are taken, override via
`SRS_RTMP_PORT` / `SRS_HTTP_PORT` / `SRS_API_PORT` in
`docker/.env.dev`.

### Point a camera at the local SRS

Drop this into the camera YAML. Note `http_host` and `webrtc_host`
point at *different* ports — the FLV/HLS server vs the WHEP signalling
API. Use `localhost` when the mediamtx publisher and the browser
viewer run on the same host as the dev stack.

```yaml
remote_rtmp:
  base_url: rtmp://localhost:1935/live
  stream: cam1
  http_host: localhost:18085     # HLS / FLV egress port
  webrtc_host: localhost:1985    # WHEP signalling port (http_api)
```

Restart `lab serve`. The `MediaSourceOnline` event will now include
four extra `location: "remote"` endpoints alongside the three local
mediamtx ones:

```
{"protocol":"rtmp",    "location":"remote","url":"rtmp://localhost:1935/live/cam1"},
{"protocol":"flv",     "location":"remote","url":"http://localhost:18085/live/cam1.flv"},
{"protocol":"hls",     "location":"remote","url":"http://localhost:18085/live/cam1.m3u8"},
{"protocol":"webrtc",  "location":"remote",
 "url":"http://localhost:1985/rtc/v1/whep/?app=live&stream=cam1"}
```

### Smoke test

```sh
# 1. Confirm mediamtx is pushing to SRS.
curl -s http://localhost:1985/api/v1/streams/ | jq

# 2. SRS-side probe.
ffprobe http://localhost:18085/live/cam1.flv

# 3. End-to-end via the web UI — open the team channel that owns the
#    camera; the tile should show `live` within a few seconds.
```

### Browser fallback order

The `CameraTile` component tries each advertised endpoint in turn:

1. Local mediamtx WHEP (loopback / LAN)
2. Remote SRS WHEP
3. Remote SRS HLS (H.264, via native HLS or hls.js)
4. Local mediamtx HLS

A teammate off the LAN fails step 1 fast (mediamtx port unreachable)
and falls through to step 2. If Electron/Chromium cannot establish the
SRS UDP media path, it falls through to step 3, which works over HTTP
and uses the H.264 SRS output before trying any local HEVC HLS path.
The same endpoint list is sent to owner and teammate — only the
browser's reach differs.

### Remote SRS deployment

Configure endpoints from the SRS deployment that will receive the stream.
Larger deployments commonly use separate names because the media plane and
the HTTP plane have different transport requirements:

- The ingest endpoint accepts **RTMP over TCP** from mediamtx and may also
  advertise a browser-reachable **WebRTC media** address in its SDP.
- The playback endpoint serves **HLS playlists**, HTTP-FLV, and **WHEP
  signalling**. Do not send RTMP to an HTTP-only ingress.

The reserved domains below are placeholders, not deployed Liyan Labs services:

```yaml
remote_rtmp:
  base_url: rtmp://ingest.media.example:1935/live
  stream: lab-1
  http_host: https://playback.media.example
  webrtc_host: https://playback.media.example
```

The `https://` prefix on `http_host` / `webrtc_host` is required for TLS
deployments: the engine emits `http://...` URLs by default (matching the
bare `localhost:18085` form used in dev), and browsers loaded over
HTTPS refuse to `fetch()` `http://` resources (mixed content).
Prefixing the host with `https://` switches the emitted endpoint URLs
to HTTPS. The prefix on `base_url` is unaffected (RTMP push runs from
the runner, not the browser).

Smoke-testing without a camera:

```sh
ffmpeg -re -f lavfi -i testsrc -c:v libx264 -f flv \
  rtmp://ingest.media.example:1935/live/test
# then check the stream is listed:
curl -s https://playback.media.example/api/v1/streams/ | jq
```

Replace both example domains before running the smoke test.
