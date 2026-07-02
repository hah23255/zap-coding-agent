/// Remote control HTTP server — serves a web chat UI and tunnels it publicly.
///
/// `/remote [port]` starts the server, launches a tunnel (ngrok → localhost.run),
/// and prints a URL you can open on any device to drive the current zap session.
use anyhow::{Context, Result};
use axum::{
    Router,
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
};
use std::collections::HashMap;
use std::{net::SocketAddr, process::Command, sync::Arc};
use tokio::net::TcpListener;

/// Generate a URL-safe per-session access token from the OS CSPRNG.
/// This token is the access control for the remote session — without it,
/// the `/ws` upgrade is rejected, so a leaked tunnel URL minus the token
/// cannot drive the agent.
pub fn generate_token() -> String {
    use base64::Engine;
    let mut buf = [0u8; 18];
    getrandom::getrandom(&mut buf).expect("OS RNG unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Constant-time-ish equality for the token (avoids early-exit timing leak).
fn token_matches(expected: &str, got: Option<&String>) -> bool {
    if expected.is_empty() {
        return false; // never authenticate against an empty token
    }
    let Some(got) = got else { return false };
    if expected.len() != got.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.bytes().zip(got.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

// ── HTML UI ────────────────────────────────────────────────────────────────────

const UI_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>zap remote</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{background:#1a1824;color:#d4d0e0;font-family:-apple-system,BlinkMacSystemFont,'SF Pro',system-ui,sans-serif;height:100dvh;display:flex;flex-direction:column}
#header{padding:12px 16px;border-bottom:1px solid #2a2640;display:flex;align-items:center;gap:10px}
.dot{width:9px;height:9px;border-radius:50%;background:#3d3850;transition:background .3s}
.dot.on{background:#50d280}
#title{color:#c88c30;font-weight:700;font-size:16px;letter-spacing:-.3px}
#status-text{color:#6a6480;font-size:13px;margin-left:auto}
#messages{flex:1;overflow-y:auto;padding:16px;display:flex;flex-direction:column;gap:10px;scroll-behavior:smooth}
.bubble{max-width:88%;padding:10px 14px;border-radius:14px;font-size:15px;line-height:1.55;white-space:pre-wrap;word-break:break-word}
.bubble.user{align-self:flex-end;background:#2d2840;color:#d8d4e8;border-bottom-right-radius:4px}
.bubble.bot{align-self:flex-start;background:#1e1b2c;color:#c8c4dc;border:1px solid #2d2840;border-bottom-left-radius:4px;min-width:40px}
.bubble.bot.streaming::after{content:'▋';color:#c88c30;animation:blink 1s step-end infinite}
@keyframes blink{50%{opacity:0}}
#foot{padding:12px 14px;border-top:1px solid #2a2640;display:flex;gap:8px;align-items:flex-end}
#inp{flex:1;background:#221e30;border:1px solid #3a3550;border-radius:12px;color:#d4d0e0;font-size:15px;padding:10px 14px;resize:none;outline:none;max-height:130px;line-height:1.45;font-family:inherit}
#inp:focus{border-color:#5a5070}
#inp:disabled{opacity:.45}
#btn{background:#c88c30;border:none;border-radius:12px;color:#1a1824;font-size:20px;width:44px;height:44px;cursor:pointer;flex-shrink:0;transition:opacity .15s;display:flex;align-items:center;justify-content:center}
#btn:disabled{opacity:.3;cursor:not-allowed}
code{background:#2a2640;padding:1px 6px;border-radius:4px;font-size:13px;font-family:'SF Mono',Menlo,monospace}
</style>
</head>
<body>
<div id="header">
  <div class="dot" id="dot"></div>
  <span id="title">⚡ zap remote</span>
  <span id="status-text">connecting…</span>
</div>
<div id="messages"></div>
<div id="foot">
  <textarea id="inp" rows="1" placeholder="Message zap…" disabled></textarea>
  <button id="btn" disabled>↑</button>
</div>
<script>
const msgs=document.getElementById('messages'),
      inp=document.getElementById('inp'),
      btn=document.getElementById('btn'),
      dot=document.getElementById('dot'),
      st=document.getElementById('status-text');
let ws,cur=null,busy=false;

function setReady(ok){
  dot.className='dot'+(ok?' on':'');
  st.textContent=ok?'connected':'reconnecting…';
  inp.disabled=!ok||busy;
  btn.disabled=!ok||busy;
  if(ok)inp.focus();
}

function addBubble(role,text=''){
  const d=document.createElement('div');
  d.className='bubble '+role;
  d.textContent=text;
  msgs.appendChild(d);
  msgs.scrollTop=msgs.scrollHeight;
  return d;
}

function connect(){
  const proto=location.protocol==='https:'?'wss://':'ws://';
  // Forward the ?token=… from the page URL to the WebSocket — it is the
  // access control for the session.
  ws=new WebSocket(proto+location.host+'/ws'+location.search);
  ws.onopen=()=>setReady(true);
  ws.onclose=()=>{setReady(false);setTimeout(connect,2000)};
  ws.onerror=()=>ws.close();
  ws.onmessage=e=>{
    const d=JSON.parse(e.data);
    if(d.t==='c'){
      if(!cur){cur=addBubble('bot');cur.classList.add('streaming')}
      cur.textContent+=d.v;
      msgs.scrollTop=msgs.scrollHeight;
    }else if(d.t==='d'){
      if(cur)cur.classList.remove('streaming');
      cur=null;busy=false;
      inp.disabled=false;btn.disabled=false;inp.focus();
    }
  };
}

function send(){
  const t=inp.value.trim();
  if(!t||busy||ws.readyState!==1)return;
  addBubble('user',t);
  ws.send(JSON.stringify({t:'m',v:t}));
  inp.value='';inp.style.height='';
  busy=true;inp.disabled=true;btn.disabled=true;
}

btn.onclick=send;
inp.onkeydown=e=>{if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();send()}};
inp.oninput=()=>{inp.style.height='';inp.style.height=Math.min(inp.scrollHeight,130)+'px'};
connect();
</script>
</body>
</html>"#;

// ── Axum state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    input_tx: tokio::sync::mpsc::UnboundedSender<String>,
    /// Per-session access token. Empty disables the server (defensive default).
    token: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn serve_ui(
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // Require the token even for the page, so a tokenless visitor gets nothing.
    if state.token.is_empty() || !token_matches(&state.token, params.get("token")) {
        return (StatusCode::UNAUTHORIZED, "unauthorized — missing or invalid token").into_response();
    }
    Html(UI_HTML).into_response()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // The token is the access control for the live session: reject any upgrade
    // that does not present it. A leaked tunnel URL without the token is inert.
    if state.token.is_empty() || !token_matches(&state.token, params.get("token")) {
        return (StatusCode::UNAUTHORIZED, "unauthorized — missing or invalid token").into_response();
    }
    ws.on_upgrade(|socket| handle_socket(socket, state)).into_response()
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = {
        use futures_util::StreamExt;
        socket.split()
    };

    // Subscribe to LLM chunks and done signals for this connection.
    let (mut chunk_rx, mut done_rx) = match crate::remote_channel::subscribe() {
        Some(pair) => pair,
        None       => return,
    };

    let input_tx = state.input_tx.clone();

    // Spawn: forward LLM output → WebSocket
    let out_task = tokio::spawn(async move {
        use futures_util::SinkExt;
        loop {
            tokio::select! {
                Ok(chunk) = chunk_rx.recv() => {
                    let msg: String = serde_json::json!({"t":"c","v":chunk}).to_string();
                    if sink.send(Message::Text(msg)).await.is_err() { break; }
                }
                Ok(()) = done_rx.recv() => {
                    let msg: String = serde_json::json!({"t":"d"}).to_string();
                    if sink.send(Message::Text(msg)).await.is_err() { break; }
                }
                else => break,
            }
        }
    });

    // Main: receive messages from browser → session input channel
    use futures_util::StreamExt;
    while let Some(Ok(msg)) = stream.next().await {
        if let Message::Text(text) = msg {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                if val["t"].as_str() == Some("m") {
                    if let Some(v) = val["v"].as_str() {
                        let _ = input_tx.send(v.to_string());
                    }
                }
            }
        }
    }

    out_task.abort();
}

// ── Server startup ────────────────────────────────────────────────────────────

/// Bind to `port` (0 = random), return the actual port used.
/// `token` is the per-session access control required on the page and the
/// `/ws` upgrade. An empty token refuses all connections.
pub async fn start_server(port: u16, token: String) -> Result<u16> {
    let input_tx = crate::remote_channel::input_sender()
        .context("remote_channel not activated — call remote_channel::activate() first")?;

    let state = Arc::new(AppState { input_tx, token });

    let app = Router::new()
        .route("/",   get(serve_ui))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await
        .with_context(|| format!("could not bind to port {}", port))?;
    let actual_port = listener.local_addr()?.port();

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    crate::remote_channel::set_server_abort(handle.abort_handle());

    Ok(actual_port)
}

// ── Tunnel ────────────────────────────────────────────────────────────────────

/// Try ngrok first (queries its local API on :4040). If ngrok is unavailable or the
/// public URL never becomes reachable end-to-end, return a clear error instead of
/// silently falling back to localhost.run, which has been returning unstable 502s.
pub async fn launch_tunnel(port: u16, token: &str) -> Result<String> {
    let ngrok_path = which_ngrok().context(
        "ngrok is required for /remote right now; install/authenticate ngrok because localhost.run fallback is disabled due to unstable 502 responses",
    )?;

    // Start ngrok in background.
    if let Ok(child) = tokio::process::Command::new(&ngrok_path)
        .args(["http", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        if let Some(pid) = child.id() { crate::remote_channel::set_tunnel_pid(pid); }
    }

    // Poll ngrok's local API until the tunnel is up and reachable end-to-end.
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(url) = ngrok_url().await {
            match wait_for_remote_url(&url, token, 20).await {
                Ok(()) => return Ok(url),
                Err(e) => {
                    anyhow::bail!(
                        "ngrok tunnel came up but the public /remote URL never became reachable: {e}"
                    );
                }
            }
        }
    }

    anyhow::bail!(
        "ngrok tunnel did not become available on http://127.0.0.1:4040/api/tunnels within 10s; check `ngrok http {}` manually to confirm auth/account setup",
        port
    )
}

async fn wait_for_remote_url(base_url: &str, token: &str, seconds: u64) -> Result<()> {
    let url = format!("{}/?token={}", base_url.trim_end_matches('/'), token);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut last_err = None;

    while tokio::time::Instant::now() < deadline {
        match crate::http::client().get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => last_err = Some(format!("unexpected status {}", resp.status())),
            Err(e) => last_err = Some(e.to_string()),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    anyhow::bail!(
        "public remote URL was not reachable after tunnel startup{}",
        last_err
            .map(|e| format!(": {e}"))
            .unwrap_or_default()
    )
}

pub fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        if let Ok(mut child) = Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                if stdin.write_all(text.as_bytes()).is_ok() {
                    return child.wait().map(|s| s.success()).unwrap_or(false);
                }
            }
        }
        false
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Set-Clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .ok()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(text.as_bytes()).ok()?;
                    child.wait().ok()
                } else {
                    None
                }
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        use std::io::Write;
        if let Ok(mut child) = Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                if stdin.write_all(text.as_bytes()).is_ok() {
                    return child.wait().map(|s| s.success()).unwrap_or(false);
                }
            }
        }
        if let Ok(mut child) = Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                if stdin.write_all(text.as_bytes()).is_ok() {
                    return child.wait().map(|s| s.success()).unwrap_or(false);
                }
            }
        }
        false
    }
}

fn which_ngrok() -> Result<String> {
    // Check common locations.
    for path in [
        "/opt/homebrew/bin/ngrok",
        "/usr/local/bin/ngrok",
        "/usr/bin/ngrok",
    ] {
        if std::path::Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }
    // Also try PATH via `which`.
    let out = std::process::Command::new("which").arg("ngrok").output()?;
    if out.status.success() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() { return Ok(p); }
    }
    anyhow::bail!("ngrok not found")
}

async fn ngrok_url() -> Result<String> {
    let body = crate::http::client()
        .get("http://127.0.0.1:4040/api/tunnels")
        .send()
        .await?
        .text()
        .await?;
    let val: serde_json::Value = serde_json::from_str(&body)?;
    val["tunnels"]
        .as_array()
        .and_then(|arr| arr.iter().find_map(|t| {
            if t["proto"].as_str() == Some("https") {
                t["public_url"].as_str().map(|s| s.to_string())
            } else {
                None
            }
        }))
        .context("no https tunnel in ngrok API response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_nonempty_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert!(a.len() >= 20, "token should be reasonably long: {a}");
        assert_ne!(a, b, "tokens must be unique per call");
    }

    #[test]
    fn token_match_requires_exact_value() {
        let t = generate_token();
        assert!(token_matches(&t, Some(&t)));
        assert!(!token_matches(&t, None));
        assert!(!token_matches(&t, Some(&"".to_string())));
        assert!(!token_matches(&t, Some(&format!("{t}x"))));
        assert!(!token_matches(&t, Some(&"wrong".to_string())));
    }

    #[test]
    fn empty_expected_never_matches() {
        // Defensive: an empty server token must reject everything.
        assert!(!token_matches("", Some(&"".to_string())));
    }
}
