//! A phone's view of the remote door, end to end: a real engine behind a real socket, a
//! WebSocket client that pairs, proves itself, calls the bus and streams a terminal — once
//! directly and once through a rendezvous server and the host tunnel.

use futures_util::{SinkExt, StreamExt};
use relay_core::engine::Engine;
use relay_core::socket::SocketServer;
use relay_core::{Instance, Store};
use relay_remote::direct::DirectServer;
use relay_remote::rendezvous::RendezvousServer;
use relay_remote::wire::{self, Greeting, Welcome};
use relay_remote::{Ctx, Registry};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Harness {
    _dir: tempfile::TempDir,
    _engine: Arc<Engine>,
    _socket: SocketServer,
    ctx: Arc<Ctx>,
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.db"), false).unwrap();
    let engine = Engine::new(Instance::Test, store);
    let socket = SocketServer::start_in(engine.clone(), dir.path().join("run"))
        .await
        .expect("socket door");
    let ctx = Arc::new(Ctx {
        instance: Instance::Test,
        registry_path: dir.path().join("remote.json"),
        socket_path: socket.path.clone(),
        version: "test".into(),
    });
    Harness { _dir: dir, _engine: engine, _socket: socket, ctx }
}

/// A first-come window, as `relay remote pair --no-confirm` opens; confirmation has its own
/// tests in `bridge.rs`.
fn pair_code(ctx: &Ctx) -> String {
    Registry::update(&ctx.registry_path, |r| Ok::<_, anyhow::Error>(r.begin_pair(false).code)).unwrap()
}

async fn recv_text(ws: &mut Ws) -> String {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("no line in 5s") {
            Some(Ok(Message::Text(t))) => return t.to_string(),
            Some(Ok(Message::Close(_))) | None => panic!("socket closed"),
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("socket error: {e}"),
        }
    }
}

async fn recv_json(ws: &mut Ws) -> Value {
    serde_json::from_str(&recv_text(ws).await).unwrap()
}

async fn send(ws: &mut Ws, line: String) {
    ws.send(Message::text(line)).await.unwrap();
}

async fn call(ws: &mut Ws, op: &str, payload: Value) -> Value {
    let id = uuid_v4();
    send(ws, json!({"v":1,"id":id,"actor":"user","op":op,"payload":payload}).to_string()).await;
    loop {
        let line = recv_json(ws).await;
        if line.get("id").and_then(|i| i.as_str()) == Some(&id) {
            return line;
        }
    }
}

fn uuid_v4() -> String {
    // A UUID shape the engine accepts, without pulling `uuid` into dev-deps.
    let hex = relay_remote::registry::random_hex(16);
    format!("{}-{}-4{}-a{}-{}", &hex[..8], &hex[8..12], &hex[13..16], &hex[17..20], &hex[20..32])
}

/// Pair over a fresh socket; returns the credential and the greeting.
async fn pair(url: &str, code: &str) -> (String, String, Greeting) {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("connect");
    let greeting: Greeting = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
    assert_eq!(greeting.relay, "remote");
    assert_eq!(greeting.instance, "test");
    send(&mut ws, json!({"v":1,"pair":code,"device_name":"Test Phone"}).to_string()).await;
    let welcome: Welcome = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
    assert!(welcome.ok, "{welcome:?}");
    (welcome.device.unwrap(), welcome.token.unwrap(), greeting)
}

/// Connect with a credential; returns the admitted socket.
async fn admit(url: &str, device: &str, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("connect");
    let greeting: Greeting = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
    send(
        &mut ws,
        json!({"v":1,"device":device,"proof":wire::proof(&greeting.challenge, token)}).to_string(),
    )
    .await;
    let welcome: Welcome = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
    assert!(welcome.ok, "{welcome:?}");
    assert!(welcome.token.is_none());
    ws
}

async fn exercise_bus(ws: &mut Ws) {
    let pong = call(ws, "bus.ping", json!({})).await;
    assert_eq!(pong["ok"], true, "{pong}");
    assert_eq!(pong["result"]["pong"], true);

    // A phone is the user: an agent envelope is refused here, with the engine's error shape.
    let id = uuid_v4();
    send(ws, json!({"v":1,"id":id,"actor":"agent:brisk-otter","op":"bus.ping","payload":{},"token":"nope"}).to_string()).await;
    let refused = recv_json(ws).await;
    assert_eq!(refused["id"], id);
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["code"], "bus.actor");

    // Events interleave with responses exactly as on the socket door.
    let sub = call(ws, "bus.subscribe", json!({"events":["workspace.*"]})).await;
    assert_eq!(sub["ok"], true, "{sub}");
    // The event and the response race for the wire; both must arrive, in either order.
    let dir = tempfile::tempdir().unwrap();
    let id = uuid_v4();
    send(ws, json!({"v":1,"id":id,"actor":"user","op":"workspace.create","payload":{"path": dir.path(), "name": "phone"}}).to_string()).await;
    let (mut saw_event, mut saw_response) = (false, false);
    while !(saw_event && saw_response) {
        let line = recv_json(ws).await;
        if line.get("ev").and_then(|e| e.as_str()) == Some("workspace.changed") {
            assert_eq!(line["payload"]["name"], "phone");
            saw_event = true;
        } else if line.get("id").and_then(|i| i.as_str()) == Some(&id) {
            assert_eq!(line["ok"], true, "{line}");
            saw_response = true;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_pairs_proves_itself_and_uses_the_bus_directly() {
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let url = format!("ws://{}", door.local_addr);

    // Nothing paired and no window open: the door shows the network nothing at all.
    assert!(tokio_tungstenite::connect_async(url.as_str()).await.is_err(), "an unused door answered");

    // A window is open, but a proof for an unknown device is turned away before the engine hears it.
    let code = pair_code(&h.ctx);
    {
        let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.unwrap();
        let _greeting = recv_text(&mut ws).await;
        send(&mut ws, json!({"v":1,"device":"nobody","proof":"00"}).to_string()).await;
        let welcome: Welcome = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
        assert_eq!(welcome.error.as_deref(), Some("auth.unknown_device"));
    }

    let (device, token, greeting) = pair(&url, &code).await;
    assert_eq!(greeting.host, Registry::load(&h.ctx.registry_path).unwrap().host_name);

    // The code is spent; a second phone with the same code is refused.
    {
        let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.unwrap();
        let _ = recv_text(&mut ws).await;
        send(&mut ws, json!({"v":1,"pair":code,"device_name":"Imposter"}).to_string()).await;
        let welcome: Welcome = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
        assert_eq!(welcome.error.as_deref(), Some("pair.invalid"));
    }

    let mut ws = admit(&url, &device, &token).await;
    exercise_bus(&mut ws).await;

    // A wrong proof is a wrong proof, even for a known device.
    {
        let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.unwrap();
        let _ = recv_text(&mut ws).await;
        send(&mut ws, json!({"v":1,"device":device,"proof":wire::proof("stale", &token)}).to_string()).await;
        let welcome: Welcome = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
        assert_eq!(welcome.error.as_deref(), Some("auth.bad_proof"));
    }

    // Revoking the device ends its access on the next connection. (A second phone keeps the
    // door answering, so the refusal is the device's, not the door's.)
    let _second = pair(&url, &pair_code(&h.ctx)).await;
    assert!(Registry::update(&h.ctx.registry_path, |r| Ok::<_, anyhow::Error>(r.revoke(&device))).unwrap());
    {
        let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.unwrap();
        let greeting: Greeting = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
        send(&mut ws, json!({"v":1,"device":device,"proof":wire::proof(&greeting.challenge, &token)}).to_string()).await;
        let welcome: Welcome = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
        assert_eq!(welcome.error.as_deref(), Some("auth.unknown_device"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_info_answers_plain_http_without_a_handshake() {
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn get_info(addr: SocketAddr) -> String {
        let mut tcp = TcpStream::connect(addr).await.unwrap();
        tcp.write_all(b"GET /info HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let mut buf = String::new();
        // A door that closes without reading the request may reset the connection instead.
        let _ = tcp.read_to_string(&mut buf).await;
        buf
    }
    // Unused, the door does not even say what it is.
    assert_eq!(get_info(door.local_addr).await, "");

    pair_code(&h.ctx);
    let buf = get_info(door.local_addr).await;
    assert!(buf.starts_with("HTTP/1.1 200 OK"));
    // No web page may read it from the person's browser.
    assert!(!buf.to_ascii_lowercase().contains("access-control-allow-origin"), "{buf}");
    let body = buf.split("\r\n\r\n").nth(1).unwrap();
    let info: Value = serde_json::from_str(body).unwrap();
    assert_eq!(info["relay"], "remote");
    assert_eq!(info["instance"], "test");
    assert_eq!(info["pairing_open"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_web_page_cannot_open_the_door_but_the_phone_can() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    pair_code(&h.ctx);
    let url = format!("ws://{}", door.local_addr);
    let with_origin = |origin: &str| {
        let mut req = url.as_str().into_client_request().unwrap();
        req.headers_mut().insert("origin", origin.parse().unwrap());
        req
    };
    // A browser on another site.
    assert!(tokio_tungstenite::connect_async(with_origin("https://evil.example")).await.is_err());
    // React Native sends the URL's own origin.
    let (mut ws, _) = tokio_tungstenite::connect_async(with_origin(&format!("http://{}", door.local_addr)))
        .await
        .expect("same-origin upgrade");
    let greeting: Greeting = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
    assert_eq!(greeting.relay, "remote");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_address_cannot_take_every_unproven_slot() {
    use tokio::io::AsyncReadExt;
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "0.0.0.0:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let code = pair_code(&h.ctx);
    let port = door.local_addr.port();

    // A peer that opens sockets and says nothing gets a few, then is refused at once.
    let mut idle = Vec::new();
    for _ in 0..4 {
        idle.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut refused = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut byte = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(2), refused.read(&mut byte))
        .await
        .expect("the extra connection was held open")
        .unwrap_or(0);
    assert_eq!(n, 0);

    // A phone on another address still pairs.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.2:0".parse().unwrap()).unwrap();
    let stream = socket.connect(format!("127.0.0.1:{port}").parse().unwrap()).await.unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}"), MaybeTlsStream::Plain(stream))
        .await
        .unwrap();
    let _greeting = recv_text(&mut ws).await;
    send(&mut ws, json!({"v":1,"pair":code,"device_name":"Phone"}).to_string()).await;
    let welcome: Welcome = serde_json::from_str(&recv_text(&mut ws).await).unwrap();
    assert!(welcome.ok, "{welcome:?}");
    drop(idle);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_phone_reaches_the_engine_through_a_rendezvous() {
    let h = harness().await;
    let server = RendezvousServer::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let (rendezvous, code) = Registry::update(&h.ctx.registry_path, |r| {
        let rendezvous = r.set_rendezvous(&format!("ws://{}", server.local_addr)).clone();
        Ok::<_, anyhow::Error>((rendezvous, r.begin_pair(false).code))
    })
    .unwrap();
    let join = relay_remote::tunnel::join_url(&rendezvous);

    // No host yet: a phone that joins is told so and closed.
    {
        let (mut ws, _) = tokio_tungstenite::connect_async(join.as_str()).await.unwrap();
        let first = recv_json(&mut ws).await;
        assert_eq!(first["error"], "host.offline");
    }

    // The wrong secret cannot host the room.
    {
        let bad = format!("ws://{}/host/{}?secret=wrong", server.local_addr, rendezvous.room);
        assert!(tokio_tungstenite::connect_async(bad.as_str()).await.is_err());
    }

    let tunnel = relay_remote::tunnel::spawn(h.ctx.clone(), rendezvous.clone());
    // Wait for the host to be in the room.
    for _ in 0..50 {
        let mut tcp = TcpStream::connect(server.local_addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tcp.write_all(b"GET /health HTTP/1.1\r\n\r\n").await.unwrap();
        let mut buf = String::new();
        tcp.read_to_string(&mut buf).await.unwrap();
        if buf.contains("\"hosts\":1") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let (device, token, _) = pair(&join, &code).await;
    let mut a = admit(&join, &device, &token).await;
    exercise_bus(&mut a).await;

    // Two phones at once are two lanes, each with its own engine connection.
    let mut b = admit(&join, &device, &token).await;
    let pong = call(&mut b, "bus.ping", json!({})).await;
    assert_eq!(pong["ok"], true);
    let pong = call(&mut a, "bus.ping", json!({})).await;
    assert_eq!(pong["ok"], true);

    tunnel.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_terminal_streams_to_the_phone() {
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let url = format!("ws://{}", door.local_addr);
    let code = pair_code(&h.ctx);
    let (device, token, _) = pair(&url, &code).await;
    let mut ws = admit(&url, &device, &token).await;

    // A session that does not exist is the engine's refusal, relayed verbatim.
    let missing = call(&mut ws, "session.attach", json!({"session":"no-such"})).await;
    assert_eq!(missing["ok"], false);
    assert_eq!(missing["error"]["kind"], "not_found");

    // A minimal project so a session can exist; a provider that is `cat` so it echoes.
    let repo = tempfile::tempdir().unwrap();
    let out = std::process::Command::new("git").args(["-C", repo.path().to_str().unwrap(), "init", "-q"]).output().unwrap();
    assert!(out.status.success());
    let ws_dir = repo.path().parent().unwrap();
    let created = call(&mut ws, "workspace.create", json!({"path": ws_dir})).await;
    assert_eq!(created["ok"], true, "{created}");
    let project = call(&mut ws, "project.add", json!({"workspace_id": created["result"]["id"], "path": repo.path()})).await;
    assert_eq!(project["ok"], true, "{project}");
    let listed = call(&mut ws, "session.list", json!({})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["result"]["sessions"].as_array().map(|s| s.len()), Some(0));

    // The phone's "New request": exactly the payloads the app sends, so a schema drift on
    // either side fails here rather than on a phone. `start: false` stages the dispatch
    // without needing a provider binary on the test machine.
    let project_id = project["result"]["id"].clone();
    let task = call(&mut ws, "task.create", json!({"project_id": project_id, "title": "Fix the flaky login test", "body": "Fix the flaky login test\nand explain what was wrong."})).await;
    assert_eq!(task["ok"], true, "{task}");
    let dispatched = call(&mut ws, "task.dispatch", json!({
        "task_id": task["result"]["id"],
        "create": {"project_id": project_id, "provider": "claude", "role": "builder"},
        "start": false,
    })).await;
    assert_eq!(dispatched["ok"], true, "{dispatched}");
    let name = dispatched["result"]["session"]["name"].as_str().unwrap().to_string();
    assert_eq!(dispatched["result"]["task"]["column"], "active");

    // The one-off shape: a session with the text as its opening prompt.
    let created = call(&mut ws, "session.create", json!({"project_id": project_id, "provider": "codex", "role": "docs", "prompt": "Write the README."})).await;
    assert_eq!(created["ok"], true, "{created}");

    // Mail to the dispatched session, as the terminal's "mail" action sends it.
    let mailed = call(&mut ws, "mailbox.send", json!({"project_id": project_id, "to": name, "text": "Also update the changelog.", "priority": true})).await;
    assert_eq!(mailed["ok"], true, "{mailed}");

    let listed = call(&mut ws, "session.list", json!({})).await;
    assert_eq!(listed["result"]["sessions"].as_array().map(|s| s.len()), Some(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_fits_a_terminal_to_itself_and_hands_it_back() {
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let url = format!("ws://{}", door.local_addr);
    let code = pair_code(&h.ctx);
    let (device, token, _) = pair(&url, &code).await;
    let mut ws = admit(&url, &device, &token).await;

    // A live PTY: a provider that is a shell script waiting on its input.
    let repo = tempfile::tempdir().unwrap();
    let out = std::process::Command::new("git").args(["-C", repo.path().to_str().unwrap(), "init", "-q"]).output().unwrap();
    assert!(out.status.success());
    let provider = repo.path().join("fake-claude.sh");
    std::fs::write(&provider, "#!/bin/sh\necho hello-from-pty\nwhile IFS= read -r line; do echo \"echo:$line\"; done\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&provider, std::fs::Permissions::from_mode(0o755)).unwrap();
    let set = call(&mut ws, "settings.set", json!({"path": "providers.claude.path", "value": provider})).await;
    assert_eq!(set["ok"], true, "{set}");
    let created = call(&mut ws, "workspace.create", json!({"path": repo.path().parent().unwrap()})).await;
    assert_eq!(created["ok"], true, "{created}");
    let project = call(&mut ws, "project.add", json!({"workspace_id": created["result"]["id"], "path": repo.path()})).await;
    assert_eq!(project["ok"], true, "{project}");
    let session = call(&mut ws, "session.create", json!({"project_id": project["result"]["id"], "provider": "claude"})).await;
    assert_eq!(session["ok"], true, "{session}");
    let name = session["result"]["name"].as_str().unwrap().to_string();
    let spawned = call(&mut ws, "session.spawn", json!({"session": name})).await;
    assert_eq!(spawned["ok"], true, "{spawned}");

    // The terminal screen's opening read: the text, where it ends, and the PC's size.
    let back = call(&mut ws, "session.scrollback", json!({"session": name, "lines": 400})).await;
    assert_eq!(back["ok"], true, "{back}");
    let (cols, rows) = (back["result"]["cols"].clone(), back["result"]["rows"].clone());
    assert_eq!((cols.as_u64(), rows.as_u64()), (Some(120), Some(40)));

    // Fit to the phone for as long as it looks: turning sideways refits, and leaving (the
    // screen detaching) hands the PC's size back.
    let size = |back: &Value| (back["result"]["cols"].as_u64(), back["result"]["rows"].as_u64());
    let attached = call(&mut ws, "session.attach", json!({"session": name, "epoch": back["result"]["epoch"], "from_seq": back["result"]["seq"]})).await;
    assert_eq!(attached["ok"], true, "{attached}");
    for (fit_cols, fit_rows) in [(52, 38), (96, 20)] {
        let fit = call(&mut ws, "session.resize", json!({"session": name, "cols": fit_cols, "rows": fit_rows, "until_detach": true})).await;
        assert_eq!(fit["ok"], true, "{fit}");
        let back = call(&mut ws, "session.scrollback", json!({"session": name, "lines": 400})).await;
        assert_eq!(size(&back), (Some(fit_cols), Some(fit_rows)));
    }
    let detached = call(&mut ws, "session.detach", json!({"session": name})).await;
    assert_eq!(detached["ok"], true, "{detached}");
    let back = call(&mut ws, "session.scrollback", json!({"session": name, "lines": 400})).await;
    assert_eq!(size(&back), (cols.as_u64(), rows.as_u64()));

    // "Fit to phone" off: the PC's size, set for good, which ends the loan.
    let fit = call(&mut ws, "session.resize", json!({"session": name, "cols": 52, "rows": 38, "until_detach": true})).await;
    assert_eq!(fit["ok"], true, "{fit}");
    let wide = call(&mut ws, "session.resize", json!({"session": name, "cols": cols, "rows": rows})).await;
    assert_eq!(wide["ok"], true, "{wide}");

    // A phone that drops mid-look hands the size back too: the bridge's socket closes with it.
    let fit = call(&mut ws, "session.resize", json!({"session": name, "cols": 52, "rows": 38, "until_detach": true})).await;
    assert_eq!(fit["ok"], true, "{fit}");
    drop(ws);
    let mut ws = admit(&url, &device, &token).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let back = call(&mut ws, "session.scrollback", json!({"session": name, "lines": 400})).await;
        if size(&back) == (cols.as_u64(), rows.as_u64()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the size never came back: {back}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let closed = call(&mut ws, "session.close", json!({"session": name})).await;
    assert_eq!(closed["ok"], true, "{closed}");
}

/// The server's `/health` body.
async fn health(addr: SocketAddr) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    tcp.write_all(b"GET /health HTTP/1.1\r\n\r\n").await.unwrap();
    let mut buf = String::new();
    tcp.read_to_string(&mut buf).await.unwrap();
    serde_json::from_str(buf.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

/// Poll `/health` until `hosts` reads `want`, or fail after a few seconds.
async fn await_hosts(addr: SocketAddr, want: u64) {
    for _ in 0..100 {
        if health(addr).await["hosts"] == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the rendezvous never listed {want} host(s): {}", health(addr).await);
}

/// A bare host socket for a room of the test's own making: no engine, no tunnel.
async fn raw_host(addr: SocketAddr, secret: &str) -> Ws {
    let room = relay_remote::registry::room_for(secret);
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/host/{room}?secret={secret}"))
        .await
        .expect("host upgrade");
    ws
}

/// Whether the socket is closed (or errors) within `within`, skipping anything it still says.
async fn closes_within(ws: &mut Ws, within: Duration) -> bool {
    tokio::time::timeout(within, async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await
    .is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_a_connected_phone_closes_its_connection() {
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let url = format!("ws://{}", door.local_addr);
    let (device, token, _) = pair(&url, &pair_code(&h.ctx)).await;
    let (other, other_token, _) = pair(&url, &pair_code(&h.ctx)).await;
    let mut ws = admit(&url, &device, &token).await;
    let mut kept = admit(&url, &other, &other_token).await;
    assert_eq!(call(&mut ws, "bus.ping", json!({})).await["ok"], true);

    // Unrelated writes (another phone's `last_seen`) leave it connected.
    let _third = admit(&url, &other, &other_token).await;
    assert_eq!(call(&mut ws, "bus.ping", json!({})).await["ok"], true);

    // `relay remote revoke`, from another process: only the file changes.
    assert!(Registry::update(&h.ctx.registry_path, |r| Ok::<_, anyhow::Error>(r.revoke(&device))).unwrap());
    assert!(closes_within(&mut ws, Duration::from_secs(5)).await, "a revoked phone kept its connection");
    // The other phone is untouched.
    assert_eq!(call(&mut kept, "bus.ping", json!({})).await["ok"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_joiner_message_drops_that_joiner_not_the_tunnel() {
    let h = harness().await;
    let server = RendezvousServer::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let (rendezvous, code) = Registry::update(&h.ctx.registry_path, |r| {
        let rendezvous = r.set_rendezvous(&format!("ws://{}", server.local_addr)).clone();
        Ok::<_, anyhow::Error>((rendezvous, r.begin_pair(false).code))
    })
    .unwrap();
    let join = relay_remote::tunnel::join_url(&rendezvous);
    let tunnel = relay_remote::tunnel::spawn(h.ctx.clone(), rendezvous.clone());
    await_hosts(server.local_addr, 1).await;
    let (device, token, _) = pair(&join, &code).await;
    let mut phone = admit(&join, &device, &token).await;
    assert_eq!(call(&mut phone, "bus.ping", json!({})).await["ok"], true);

    // A stranger who knows the room id, with no credential: one message over the phone limit,
    // and one under it that JSON escaping grows past what the host socket carries.
    for payload in ["x".repeat(9 << 20), "\u{1}".repeat(3 << 20)] {
        let (mut stranger, _) = tokio_tungstenite::connect_async(join.as_str()).await.unwrap();
        let _greeting = recv_text(&mut stranger).await;
        let _ = stranger.send(Message::text(payload)).await;
        assert!(closes_within(&mut stranger, Duration::from_secs(10)).await, "the oversized joiner stayed");
    }

    // The paired phone's lane — and so the PC's tunnel — never noticed.
    assert_eq!(call(&mut phone, "bus.ping", json!({})).await["ok"], true);
    assert_eq!(health(server.local_addr).await["hosts"], 1);
    tunnel.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replaced_host_dropping_late_leaves_the_new_host_in_the_room() {
    let server = RendezvousServer::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let secret = "a-secret-of-the-test's-own";
    let room = relay_remote::registry::room_for(secret);
    let old = raw_host(server.local_addr, secret).await;
    await_hosts(server.local_addr, 1).await;
    // The PC reconnects (its old socket half-open), then the old socket finally drops.
    let mut new = raw_host(server.local_addr, secret).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(old);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(health(server.local_addr).await["hosts"], 1, "the stale host's exit cleared the room");

    // A phone that joins now reaches the new host.
    let (_phone, _) = tokio_tungstenite::connect_async(format!("ws://{}/join/{room}", server.local_addr))
        .await
        .unwrap();
    let line = recv_json(&mut new).await;
    assert_eq!(line["t"], "open", "{line}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silent_hosts_and_phones_are_dropped_and_live_ones_kept() {
    let limits = relay_remote::rendezvous::Limits {
        host_silence: Duration::from_millis(600),
        phone_silence: Duration::from_millis(600),
        ping_every: Duration::from_millis(150),
        ..Default::default()
    };
    let server = RendezvousServer::bind_with("127.0.0.1:0".parse::<SocketAddr>().unwrap(), limits)
        .await
        .unwrap();
    let addr = server.local_addr;

    // A host that keeps reading answers the server's pings and stays; one whose machine
    // vanished (it never reads or writes again) is dropped and its room empties.
    let (gone_secret, live_secret) = ("vanished-pc", "live-pc");
    let _gone = raw_host(addr, gone_secret).await;
    let mut live = raw_host(addr, live_secret).await;
    let (opened_tx, mut opened) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let reader = tokio::spawn(async move {
        while let Some(Ok(msg)) = live.next().await {
            if let Message::Text(t) = msg {
                let _ = opened_tx.send(serde_json::from_str(&t).unwrap());
            }
        }
    });
    await_hosts(addr, 2).await;
    await_hosts(addr, 1).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(health(addr).await["hosts"], 1, "a live host was dropped");

    // The vanished PC's room answers host.offline straight away instead of hanging.
    let gone_room = relay_remote::registry::room_for(gone_secret);
    let (mut late, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/join/{gone_room}")).await.unwrap();
    assert_eq!(recv_json(&mut late).await["error"], "host.offline");

    // A phone that joins the live room and then goes silent (never reading, never sending)
    // loses its lane, and the host hears it closed.
    let live_room = relay_remote::registry::room_for(live_secret);
    let (_silent, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/join/{live_room}")).await.unwrap();
    let open = tokio::time::timeout(Duration::from_secs(5), opened.recv()).await.unwrap().unwrap();
    assert_eq!(open["t"], "open");
    let close = tokio::time::timeout(Duration::from_secs(5), opened.recv()).await.expect("the silent phone kept its lane").unwrap();
    assert_eq!(close["t"], "close");
    assert_eq!(close["c"], open["c"]);
    reader.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_pairing_is_announced_to_the_desktop() {
    let h = harness().await;
    let door = DirectServer::bind(h.ctx.clone(), "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let url = format!("ws://{}", door.local_addr);
    let (device, token, _) = pair(&url, &pair_code(&h.ctx)).await;
    // A client listening as the desktop does.
    let mut desktop = admit(&url, &device, &token).await;
    let sub = call(&mut desktop, "bus.subscribe", json!({"events":["ui.toast"]})).await;
    assert_eq!(sub["ok"], true, "{sub}");

    let (second, _, _) = pair(&url, &pair_code(&h.ctx)).await;
    loop {
        let line = recv_json(&mut desktop).await;
        if line["ev"] == "ui.toast" {
            let text = line["payload"]["text"].as_str().unwrap();
            assert!(text.contains("Test Phone") && text.contains(&second) && text.contains("direct 127.0.0.1"), "{text}");
            break;
        }
    }
}
