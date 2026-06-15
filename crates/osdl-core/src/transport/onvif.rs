//! ONVIF transport — HTTP/SOAP control plane for IP cameras.
//!
//! Unlike the byte-oriented transports (MQTT serial, direct serial, TCP),
//! ONVIF speaks HTTP+SOAP. We can't shoehorn SOAP into the
//! `send(&[u8])` byte pipeline literally, so the convention is:
//!
//!   * `send(bytes)` expects the bytes to be the JSON envelope produced
//!     by [`crate::adapter::onvif::OnvifAdapter::encode_command`].
//!   * The transport parses the envelope, dispatches the matching SOAP /
//!     HTTP call, and pushes the JSON-serialized result back through
//!     `rx_tx` so the engine's standard `handle_transport_rx` →
//!     `decode_response` → `DeviceStatus` path lights up.
//!
//! This is a deliberate compromise: we encode JSON in the transport and
//! decode it in the adapter, just to fit the byte-oriented `Transport`
//! trait. It keeps the engine's command-routing code uniform across
//! transports without inventing a parallel control plane for cameras.
//!
//! Only the ONVIF operations we use are implemented (PTZ ContinuousMove +
//! Stop, Media GetSnapshotUri, PTZ GotoPreset). Targets ONVIF Media v1;
//! cameras that publish only Media2 will need a follow-up.
//!
//! Auth uses WS-Security UsernameToken with PasswordDigest — broad LCD
//! across IP-camera firmware. HTTP Digest is *not* implemented; if a
//! camera demands it, that's a separate fix. We accept self-signed certs
//! deliberately (`danger_accept_invalid_certs(true)`): ONVIF is LAN-only
//! by deployment assumption, and forcing a CA chain on a closet camera
//! is a worse failure mode than tolerating its dev cert.

use super::{Transport, TransportRx};
use async_trait::async_trait;
use base64::Engine as _;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct OnvifTransport {
    inner: Arc<Inner>,
}

struct Inner {
    /// Camera id used for transport_id and snapshot directory naming.
    camera_id: String,
    /// `onvif:cam1` etc. — what the engine routes by.
    transport_id: String,
    onvif_url: String,
    username: String,
    password: String,
    profile_token_override: Option<String>,
    /// Resolved on first PTZ/snapshot call. RwLock so multiple commands
    /// can read once cached without serialization.
    discovered: RwLock<Option<DiscoveredEndpoints>>,
    /// Where to drop snapshot JPEGs. The engine creates `<root>/<cam_id>/`.
    snapshot_root: PathBuf,
    /// Public URL prefix that maps to `snapshot_root` for callers to fetch
    /// the resulting JPEG. Optional — when unset, only `snapshot_path` is
    /// surfaced (a `file://` URL is also returned as a convenience).
    snapshot_url_base: Option<String>,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    http: reqwest::Client,
    /// Latest auto-stop task (if any). A new `ptz_move` aborts the prior
    /// one so two consecutive moves don't have the first move's auto-stop
    /// fire mid-second-move and freeze the camera unexpectedly. Held in a
    /// Mutex<Option<JoinHandle>> rather than an atomic so the abort can
    /// happen under the same critical section that installs the new task.
    auto_stop: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Clone)]
struct DiscoveredEndpoints {
    media_url: String,
    ptz_url: String,
    profile_token: String,
}

impl OnvifTransport {
    pub fn new(
        camera_id: String,
        onvif_url: String,
        username: String,
        password: String,
        profile_token: Option<String>,
        snapshot_root: PathBuf,
        snapshot_url_base: Option<String>,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Self {
        let transport_id = transport_id_for(&camera_id);
        // Liberal timeouts: cameras over wifi can stall briefly during
        // motion, but never longer than a few seconds for these ops.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .danger_accept_invalid_certs(true)
            .build()
            .expect("build reqwest client");
        Self {
            inner: Arc::new(Inner {
                camera_id,
                transport_id,
                onvif_url,
                username,
                password,
                profile_token_override: profile_token,
                discovered: RwLock::new(None),
                snapshot_root,
                snapshot_url_base,
                rx_tx,
                http,
                auto_stop: Mutex::new(None),
            }),
        }
    }

    /// Resolve the camera's PTZ + Media service URLs and a usable profile
    /// token. Cached after first success.
    async fn endpoints(&self) -> Result<DiscoveredEndpoints, String> {
        if let Some(d) = self.inner.discovered.read().await.clone() {
            return Ok(d);
        }

        let caps = self
            .soap(&self.inner.onvif_url, BODY_GET_CAPABILITIES)
            .await
            .map_err(|e| format!("GetCapabilities: {e}"))?;
        // Don't fall back to onvif_url for media_url. If GetCapabilities
        // parsing fails, send a clear error: GetProfiles must hit /Media,
        // not /device_service, and pretending otherwise produces a 404
        // far from where the misconfiguration actually lives.
        let media_url = extract_xaddr(&caps, "Media").ok_or_else(|| {
            "GetCapabilities response missing <tt:Media><tt:XAddr>".to_string()
        })?;
        let ptz_url = extract_xaddr(&caps, "PTZ").unwrap_or_else(|| media_url.clone());

        let token = match self.inner.profile_token_override.clone() {
            Some(t) => t,
            None => {
                let profiles = self
                    .soap(&media_url, BODY_GET_PROFILES)
                    .await
                    .map_err(|e| format!("GetProfiles: {e}"))?;
                first_profile_token(&profiles)
                    .ok_or_else(|| "no profile tokens in GetProfiles response".to_string())?
            }
        };

        let d = DiscoveredEndpoints {
            media_url,
            ptz_url,
            profile_token: token,
        };
        *self.inner.discovered.write().await = Some(d.clone());
        Ok(d)
    }

    /// Send a SOAP request and return the response body. Handles
    /// WS-Security UsernameToken auth automatically.
    async fn soap(&self, endpoint: &str, body_template: &str) -> Result<String, String> {
        let security = ws_security_header(&self.inner.username, &self.inner.password);
        let envelope = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"
            xmlns:tds="http://www.onvif.org/ver10/device/wsdl"
            xmlns:trt="http://www.onvif.org/ver10/media/wsdl"
            xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl"
            xmlns:tt="http://www.onvif.org/ver10/schema">
  <s:Header>{security}</s:Header>
  <s:Body>{body_template}</s:Body>
</s:Envelope>"#
        );

        // SOAP 1.2 carries the action via Content-Type's `action`
        // parameter, not a separate header. Most ONVIF cameras accept
        // either; sending an empty SOAPAction is the safest LCD across
        // firmware vintages.
        let resp = self
            .inner
            .http
            .post(endpoint)
            .header("Content-Type", "application/soap+xml; charset=utf-8")
            .header("SOAPAction", "\"\"")
            .body(envelope)
            .send()
            .await
            .map_err(|e| format!("HTTP send: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {}", first_chars(&text, 256)));
        }
        Ok(text)
    }

    /// Dispatch a parsed envelope. Returns the result envelope to be sent
    /// back through rx_tx.
    async fn dispatch(&self, op: &str, args: &Value) -> Value {
        match self.dispatch_inner(op, args).await {
            Ok(data) => json!({ "op": op, "ok": true, "data": data }),
            Err(e) => {
                log::warn!("onvif {} failed on {}: {}", op, self.inner.camera_id, e);
                json!({ "op": op, "ok": false, "error": e })
            }
        }
    }

    async fn dispatch_inner(&self, op: &str, args: &Value) -> Result<Value, String> {
        match op {
            "ptz_move" => self.do_ptz_move(args).await,
            "ptz_stop" => self.do_ptz_stop().await,
            "ptz_preset_goto" => self.do_ptz_preset_goto(args).await,
            "snapshot" => self.do_snapshot().await,
            other => Err(format!("unknown op '{other}'")),
        }
    }

    async fn do_ptz_move(&self, args: &Value) -> Result<Value, String> {
        let pan = args.get("pan").and_then(Value::as_f64).unwrap_or(0.0);
        let tilt = args.get("tilt").and_then(Value::as_f64).unwrap_or(0.0);
        let zoom = args.get("zoom").and_then(Value::as_f64).unwrap_or(0.0);
        let duration_ms = args.get("duration_ms").and_then(Value::as_u64).unwrap_or(500);

        let ep = self.endpoints().await?;
        let body = format!(
            r#"<tptz:ContinuousMove>
  <tptz:ProfileToken>{token}</tptz:ProfileToken>
  <tptz:Velocity>
    <tt:PanTilt x="{pan}" y="{tilt}" xmlns:tt="http://www.onvif.org/ver10/schema"/>
    <tt:Zoom x="{zoom}" xmlns:tt="http://www.onvif.org/ver10/schema"/>
  </tptz:Velocity>
</tptz:ContinuousMove>"#,
            token = xml_escape(&ep.profile_token),
            pan = pan,
            tilt = tilt,
            zoom = zoom,
        );
        self.soap(&ep.ptz_url, &body).await?;

        // Auto-stop after the requested duration. We don't await — the
        // CLI caller wants a fast PENDING reply, and the camera will
        // happily keep moving until we stop it. Replace any in-flight
        // auto-stop so back-to-back moves don't trip each other up.
        let this = self.clone();
        let ptz_url = ep.ptz_url.clone();
        let token = ep.profile_token.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(duration_ms)).await;
            let body = format!(
                r#"<tptz:Stop>
  <tptz:ProfileToken>{tok}</tptz:ProfileToken>
  <tptz:PanTilt>true</tptz:PanTilt>
  <tptz:Zoom>true</tptz:Zoom>
</tptz:Stop>"#,
                tok = xml_escape(&token),
            );
            if let Err(e) = this.soap(&ptz_url, &body).await {
                log::warn!("ptz auto-stop on {}: {}", this.inner.camera_id, e);
            }
        });
        if let Some(prior) = self.inner.auto_stop.lock().await.replace(handle) {
            prior.abort();
        }

        Ok(json!({
            "pan": pan, "tilt": tilt, "zoom": zoom, "duration_ms": duration_ms,
        }))
    }

    async fn do_ptz_stop(&self) -> Result<Value, String> {
        // Cancel any auto-stop scheduled by a prior ptz_move so it
        // doesn't fire seconds later and undo a fresh manual move.
        if let Some(prior) = self.inner.auto_stop.lock().await.take() {
            prior.abort();
        }
        let ep = self.endpoints().await?;
        let body = format!(
            r#"<tptz:Stop>
  <tptz:ProfileToken>{token}</tptz:ProfileToken>
  <tptz:PanTilt>true</tptz:PanTilt>
  <tptz:Zoom>true</tptz:Zoom>
</tptz:Stop>"#,
            token = xml_escape(&ep.profile_token),
        );
        self.soap(&ep.ptz_url, &body).await?;
        Ok(json!({}))
    }

    async fn do_ptz_preset_goto(&self, args: &Value) -> Result<Value, String> {
        let preset = args
            .get("preset")
            .and_then(Value::as_str)
            .ok_or("ptz_preset_goto: missing preset")?;
        let ep = self.endpoints().await?;
        let body = format!(
            r#"<tptz:GotoPreset>
  <tptz:ProfileToken>{token}</tptz:ProfileToken>
  <tptz:PresetToken>{preset}</tptz:PresetToken>
</tptz:GotoPreset>"#,
            token = xml_escape(&ep.profile_token),
            preset = xml_escape(preset),
        );
        self.soap(&ep.ptz_url, &body).await?;
        Ok(json!({"preset": preset}))
    }

    async fn do_snapshot(&self) -> Result<Value, String> {
        let ep = self.endpoints().await?;
        let body = format!(
            r#"<trt:GetSnapshotUri>
  <trt:ProfileToken>{token}</trt:ProfileToken>
</trt:GetSnapshotUri>"#,
            token = xml_escape(&ep.profile_token),
        );
        let resp = self.soap(&ep.media_url, &body).await?;
        let snap_uri = extract_tag(&resp, "Uri")
            .ok_or("GetSnapshotUri: no Uri in response")?;

        // Cameras commonly require Basic auth on the snapshot URI even
        // when the SOAP endpoint accepted UsernameToken.
        let req = self.inner.http.get(&snap_uri);
        let req = if !self.inner.username.is_empty() {
            req.basic_auth(&self.inner.username, Some(&self.inner.password))
        } else {
            req
        };
        let resp = req.send().await.map_err(|e| format!("snapshot GET: {e}"))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("snapshot read: {e}"))?;
        if !status.is_success() {
            return Err(format!("snapshot HTTP {status}"));
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let cam_dir = self.inner.snapshot_root.join(&self.inner.camera_id);
        std::fs::create_dir_all(&cam_dir)
            .map_err(|e| format!("create snapshot dir: {e}"))?;
        let path = cam_dir.join(format!("{now_ms}.jpg"));
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("write snapshot: {e}"))?;

        let path_str = path.display().to_string();
        let url = match &self.inner.snapshot_url_base {
            Some(base) => format!(
                "{}/{}/{}",
                base.trim_end_matches('/'),
                self.inner.camera_id,
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("snapshot.jpg")
            ),
            None => format!("file://{path_str}"),
        };
        Ok(json!({
            "path": path_str,
            "url": url,
            "bytes": bytes.len(),
            "source_uri": snap_uri,
        }))
    }
}

#[async_trait]
impl Transport for OnvifTransport {
    fn transport_type(&self) -> &str {
        "onvif"
    }

    fn description(&self) -> String {
        format!("ONVIF camera {} ({})", self.inner.camera_id, self.inner.onvif_url)
    }

    /// `bytes` is a JSON envelope from `OnvifAdapter::encode_command`.
    /// Dispatch on a spawned task and push the JSON result back through
    /// rx_tx so the engine's standard receive path turns it into a
    /// `DeviceStatus` event.
    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        let envelope: Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("onvif: parse envelope: {e}"))?;
        let op = envelope
            .get("op")
            .and_then(Value::as_str)
            .ok_or("onvif: envelope missing `op`")?
            .to_string();
        let args = envelope
            .get("args")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        let this = self.clone();
        let transport_id = self.inner.transport_id.clone();
        let tx = self.inner.rx_tx.clone();
        tokio::spawn(async move {
            let result = this.dispatch(&op, &args).await;
            let bytes = serde_json::to_vec(&result).unwrap_or_default();
            let _ = tx.send(TransportRx {
                transport_id,
                data: bytes,
            });
        });
        Ok(())
    }

    fn is_connected(&self) -> bool {
        // No persistent connection — every op is a fresh HTTP request.
        // The first failed send surfaces as a CommandResult error, so a
        // dropped camera shows up at action time rather than on a probe.
        true
    }

    async fn stop(&self) -> Result<(), String> {
        if let Some(prior) = self.inner.auto_stop.lock().await.take() {
            prior.abort();
        }
        Ok(())
    }
}

/// Stable transport id for a camera. Mirrors the `espnow:MAC` convention.
pub fn transport_id_for(camera_id: &str) -> String {
    format!("onvif:{camera_id}")
}

/// Build a WS-Security UsernameToken header with a SHA-1 password digest.
/// Most ONVIF cameras accept this without further negotiation.
fn ws_security_header(username: &str, password: &str) -> String {
    let nonce_bytes: [u8; 16] = rand_nonce();
    let nonce_b64 = base64::engine::general_purpose::STANDARD.encode(nonce_bytes);
    let created = format_created_now();
    let mut hasher = Sha1::new();
    hasher.update(nonce_bytes);
    hasher.update(created.as_bytes());
    hasher.update(password.as_bytes());
    let digest = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());

    format!(
        r#"<wsse:Security xmlns:wsse="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd" xmlns:wsu="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">
  <wsse:UsernameToken>
    <wsse:Username>{user}</wsse:Username>
    <wsse:Password Type="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest">{digest}</wsse:Password>
    <wsse:Nonce EncodingType="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary">{nonce}</wsse:Nonce>
    <wsu:Created>{created}</wsu:Created>
  </wsse:UsernameToken>
</wsse:Security>"#,
        user = xml_escape(username),
        digest = digest,
        nonce = nonce_b64,
        created = created,
    )
}

fn rand_nonce() -> [u8; 16] {
    // We don't have `rand` in the workspace and don't need cryptographic
    // strength — the WS-Security digest just needs the nonce to be
    // unique-ish per request. Use process time + a counter.
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let ctr = CTR.fetch_add(1, Ordering::Relaxed);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&now_nanos.to_le_bytes());
    out[8..].copy_from_slice(&ctr.to_le_bytes());
    out
}

fn format_created_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Minimal calendar conversion. Adequate for ONVIF's `wsu:Created`
/// timestamp; not for general-purpose use. All fields are unsigned —
/// negative epochs (pre-1970) aren't representable here.
fn epoch_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let s = secs % 86400;
    let h = (s / 3600) as u32;
    let mi = ((s % 3600) / 60) as u32;
    let s = (s % 60) as u32;

    // Days since 1970-01-01 to civil from Howard Hinnant.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    let y = if y < 0 { 0 } else { y as u32 };
    (y, m, d, h, mi, s)
}

const BODY_GET_CAPABILITIES: &str =
    r#"<tds:GetCapabilities><tds:Category>All</tds:Category></tds:GetCapabilities>"#;

const BODY_GET_PROFILES: &str = r#"<trt:GetProfiles/>"#;

/// Find `<*:Service><*:XAddr>...` for a named ONVIF service inside a
/// GetCapabilities response. Uses quick-xml so namespace prefixes,
/// attributes, and similarly-named siblings (e.g. Media vs.
/// MediaServiceCapabilities) don't fool us.
fn extract_xaddr(body: &str, service: &str) -> Option<String> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);
    let mut depth_in_service = 0u32;
    let mut in_xaddr = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if depth_in_service > 0 {
                    depth_in_service += 1;
                    if name == "XAddr" && depth_in_service == 2 {
                        in_xaddr = true;
                    }
                } else if name == service {
                    depth_in_service = 1;
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                if depth_in_service > 0 {
                    if in_xaddr && name == "XAddr" {
                        in_xaddr = false;
                    }
                    depth_in_service -= 1;
                }
            }
            Ok(Event::Text(t)) if in_xaddr => {
                let s = t.unescape().ok()?.into_owned();
                let s = s.trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// Find the first `<*:tag>...</*:tag>` (any namespace prefix). Used to
/// pull `Uri` out of GetSnapshotUri responses.
fn extract_tag(body: &str, tag: &str) -> Option<String> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);
    let mut in_tag = 0u32;
    let mut out = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == tag {
                    in_tag += 1;
                }
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == tag && in_tag > 0 {
                    in_tag -= 1;
                    if in_tag == 0 && !out.is_empty() {
                        return Some(out);
                    }
                }
            }
            Ok(Event::Text(t)) if in_tag > 0 => {
                if let Ok(s) = t.unescape() {
                    out.push_str(s.trim());
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    if out.is_empty() { None } else { Some(out) }
}

/// First `token="..."` attribute on a `<*:Profiles ...>` element.
fn first_profile_token(body: &str) -> Option<String> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == "Profiles" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"token" {
                            return attr
                                .unescape_value()
                                .ok()
                                .map(|c| c.into_owned());
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// Local-name part of a (possibly namespaced) tag — `tt:XAddr` → `XAddr`.
/// Returns owned because quick-xml's `name()` borrows from a temporary.
fn local_name(name: &[u8]) -> String {
    let s = std::str::from_utf8(name).unwrap_or("");
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn first_chars(s: &str, n: usize) -> &str {
    if s.len() <= n { s } else { &s[..n] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_xaddr_picks_correct_service_amid_lookalikes() {
        // The lookalike `<tt:MediaServiceCapabilities>` must NOT win over
        // `<tt:Media>`. The hand-rolled substring matcher this replaced
        // was vulnerable to that.
        let xml = r#"
<s:Envelope><s:Body>
<tds:GetCapabilitiesResponse><tds:Capabilities>
  <tt:MediaServiceCapabilities><tt:XAddr>http://wrong/Capabilities</tt:XAddr></tt:MediaServiceCapabilities>
  <tt:Media>
    <tt:XAddr>http://192.168.1.131/onvif/Media</tt:XAddr>
  </tt:Media>
  <tt:PTZ>
    <tt:XAddr>http://192.168.1.131/onvif/PTZ</tt:XAddr>
  </tt:PTZ>
</tds:Capabilities></tds:GetCapabilitiesResponse>
</s:Body></s:Envelope>"#;
        assert_eq!(
            extract_xaddr(xml, "Media").as_deref(),
            Some("http://192.168.1.131/onvif/Media")
        );
        assert_eq!(
            extract_xaddr(xml, "PTZ").as_deref(),
            Some("http://192.168.1.131/onvif/PTZ")
        );
    }

    #[test]
    fn extract_xaddr_returns_none_when_service_absent() {
        let xml = r#"<s:Envelope><s:Body><tds:Capabilities><tt:Media><tt:XAddr>x</tt:XAddr></tt:Media></tds:Capabilities></s:Body></s:Envelope>"#;
        assert!(extract_xaddr(xml, "PTZ").is_none());
    }

    #[test]
    fn extracts_first_profile_token() {
        let xml = r#"<trt:Profiles fixed="true" token="Profile_1"><tt:Name>main</tt:Name></trt:Profiles>"#;
        assert_eq!(first_profile_token(xml).as_deref(), Some("Profile_1"));
    }

    #[test]
    fn extracts_tag_handles_namespaces() {
        let xml = r#"<a><trt:GetSnapshotUriResponse><trt:MediaUri><tt:Uri>http://cam/x?stream=0</tt:Uri></trt:MediaUri></trt:GetSnapshotUriResponse></a>"#;
        assert_eq!(
            extract_tag(xml, "Uri").as_deref(),
            Some("http://cam/x?stream=0")
        );
    }

    #[test]
    fn xml_escape_handles_specials() {
        assert_eq!(xml_escape("a&b<c>d\""), "a&amp;b&lt;c&gt;d&quot;");
    }

    #[test]
    fn epoch_calendar_conversion_matches_known_dates() {
        // 2024-01-01T00:00:00Z = 1704067200
        assert_eq!(epoch_to_ymdhms(1704067200), (2024, 1, 1, 0, 0, 0));
        // 2026-06-15T12:30:45Z = 1781526645
        assert_eq!(epoch_to_ymdhms(1781526645), (2026, 6, 15, 12, 30, 45));
        // 2000-02-29 (leap day) at 00:00:00 = 951782400
        assert_eq!(epoch_to_ymdhms(951782400), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn ws_security_header_is_well_formed() {
        let h = ws_security_header("admin", "secret");
        assert!(h.contains("<wsse:Username>admin</wsse:Username>"));
        assert!(h.contains("PasswordDigest"));
        assert!(h.contains("<wsu:Created>"));
    }
}
