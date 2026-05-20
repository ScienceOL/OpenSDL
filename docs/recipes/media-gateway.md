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
osdl serve --detach \
  --instance camgw \
  --config /tmp/cam1.yaml \
  --registry $(pwd)/registry/unilabos
```

The engine validates the media source before spawning mediamtx. If the
camera URL is malformed or the binary can't be found, the server logs the
error and exits without leaving a process behind.

### 3. Discover the published URLs

```sh
osdl --instance camgw events --kinds media_source_online,media_gateway_down --json &
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
osdl --instance camgw stop
```

The engine signals mediamtx to terminate gracefully on shutdown.

## Notes

- HEVC sources are auto-transcoded to H.264 via an ffmpeg sidecar when
  `h264_transcode: true`. The transcoded output appears as the `_h264`
  suffix path.
- For remote ingest (push to SRS), uncomment the `remote_rtmp:` block.
  See `crates/osdl-core/src/media/onvif_camera.rs` for the validation
  rules — most importantly, you must have transcoding enabled to have an
  H.264 source path to republish from.
