//! `device.*` (phase 10): request-driven ADB discovery, H.264 mirrors, Gradle deploys,
//! and bounded logcat streams. Nothing starts until a bus op asks for it.

use crate::device::{mirror_server_path, DeviceWatchRuntime, MirrorRuntime, MirrorRuntimeConfig, MirrorState, RunRuntime};
use crate::engine::{Ctx, Engine, IntoBus};
use crate::handlers::workspace::get_project;
use crate::mirror;
use crate::paths::Instance;
use crate::worktree;
use relay_bus::error::BusError;
use relay_bus::ops::device::*;
use relay_bus::types::{Avd, Device, DeviceKind, Id, Run};
use relay_bus::Empty;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn register(e: &mut Engine) {
    // `adb devices` starts the adb server when it is not up: seconds of subprocess, and it used
    // to own the store mutex for all of it. One short read for the configured path, then adb with
    // nothing locked (D144).
    e.register_unlocked::<List>(|ctx, _| {
        let adb = ctx.read(adb_path)?;
        let mut devices = list_with_adb(&adb)?;
        // Whoever is using each device, so an agent knows before it tries (device_lease).
        let released = ctx.read(|conn| Ok(super::device_lease::prune(ctx.engine(), conn)))?;
        for lease in released { ctx.emit(super::device_lease::RELEASED, lease.event()); }
        for device in &mut devices {
            device.lease = ctx.engine().device_leases.holder_of(&device.serial).map(|lease| lease.view());
        }
        Ok(ListOut { devices })
    });

    e.register::<Watch>(|ctx: &mut Ctx, p| {
        if p.on {
            let adb = adb_path(ctx.tx())?;
            let runtime = {
                let mut watch = ctx.engine().device_watch.lock().unwrap();
                watch.clients += 1;
                if watch.runtime.is_none() {
                    let runtime = Arc::new(DeviceWatchRuntime::default());
                    watch.runtime = Some(runtime.clone());
                    Some(runtime)
                } else { None }
            };
            if let Some(runtime) = runtime {
                ctx.after_commit(move |engine| { std::thread::spawn(move || device_watch_worker(engine, runtime, adb)); });
            }
        } else {
            let runtime = {
                let mut watch = ctx.engine().device_watch.lock().unwrap();
                watch.clients = watch.clients.saturating_sub(1);
                if watch.clients == 0 { watch.runtime.take() } else { None }
            };
            if let Some(runtime) = runtime { runtime.stop(); }
        }
        Ok(Empty {})
    });

    // `adb devices` and `wm size` are two subprocesses, seconds when the adb server is cold;
    // both run in the prepare phase with nothing locked, each under a deadline (D149).
    e.register_staged::<MirrorStart, _>(|ctx, p| {
        let adb = ctx.read(adb_path)?;
        let device = require_device(&adb, &p.device)?;
        if device.state != "device" {
            return Err(BusError::unavailable("device.not_ready", format!("{} is {}", device.model, device.state)));
        }
        let (physical_width, physical_height) = device_size(&adb, &p.device)?;
        Ok((adb, physical_width, physical_height))
    }, |ctx: &mut Ctx, p, (adb, physical_width, physical_height)| {
        let max_size = mirror::capture_size_for(p.max_size.unwrap_or(mirror::CAPTURE_MIN));
        let bitrate = p.bitrate.unwrap_or_else(|| mirror::capture_bit_rate(max_size)).clamp(500_000, 50_000_000);
        let (width, height) = mirror::fit_size(physical_width, physical_height, max_size);
        let id = ctx.engine().next_mirror.fetch_add(1, Ordering::SeqCst);
        let scid = new_scid(id);
        if ctx.engine().instance != Instance::Test && mirror_server_path().is_none() {
            return Err(BusError::unavailable("device.mirror_server_missing", "the bundled scrcpy server is missing"));
        }
        let runtime = MirrorRuntime::new(MirrorRuntimeConfig {
            id,
            device: p.device,
            width,
            height,
            input_width: physical_width,
            input_height: physical_height,
            max_size,
            bitrate,
            scid,
            adb: adb.display().to_string(),
        });
        ctx.engine().mirrors.lock().unwrap().insert(id, runtime.clone());
        ctx.emit("mirror.changed", json!({"mirror_id":id,"device":runtime.device,"state":"starting","width":width,"height":height}));
        if ctx.engine().instance != Instance::Test {
            ctx.after_commit(move |engine| { std::thread::spawn(move || mirror_worker(engine, runtime)); });
        }
        Ok(MirrorStartOut { mirror_id: id, width, height })
    });

    e.register::<MirrorStop>(|ctx: &mut Ctx, p| {
        let runtime = ctx
            .engine()
            .mirrors
            .lock()
            .unwrap()
            .remove(&p.mirror_id)
            .ok_or_else(|| {
                BusError::not_found(
                    "device.mirror_not_found",
                    format!("no mirror {}", p.mirror_id),
                )
            })?;
        runtime.stop();
        runtime.finish(MirrorState::Stopped, None, None);
        ctx.emit(
            "mirror.changed",
            json!({"mirror_id":p.mirror_id,"device":runtime.device,"state":"stopped"}),
        );
        Ok(Empty {})
    });

    // The engine answers this from memory before it gets here (`Engine::mirror_fast_path`);
    // this handler is the fallback for the cases that path declines — an audited or
    // session-bound caller — and it touches nothing in the store either.
    e.register::<MirrorInput>(|ctx, p| {
        let runtime = mirror_by_id(ctx.engine(), p.mirror_id)?;
        send_input(&runtime, &p.event)?;
        Ok(Empty {})
    });

    e.register::<RunOp>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        // The device lease (device_lease): refuse before any adb or Gradle work when another
        // session is installing on this device, and name it.
        let requested_root = p.worktree.clone().unwrap_or_else(|| project.path.clone());
        let requested_root = std::fs::canonicalize(&requested_root).map(|path| path.display().to_string()).unwrap_or(requested_root);
        let holder = match ctx.actor_session_id() {
            Some(id) => sessions_holder(ctx, id)?,
            None => super::device_lease::worktree_holder(ctx.tx(), project.id, &requested_root)?,
        };
        super::device_lease::check(ctx, &p.device, &holder)?;
        let adb = adb_path(ctx.tx())?;
        let device = require_device(&adb, &p.device)?;
        if device.state != "device" {
            return Err(BusError::unavailable("device.not_ready", format!("{} is {}", device.model, device.state)));
        }
        let default_gradle_command = project.run_cmd.as_deref().is_none_or(|command| command.trim().is_empty());
        let gradle_init = default_gradle_command.then(gradle_init_script).transpose()?;
        let root = resolve_run_root(ctx, project.id, &project.path, p.worktree.as_deref(), p.integration_id)?;
        let project_root = std::fs::canonicalize(&project.path)
            .map_err(|error| BusError::invalid("project.path", error.to_string()))?;
        let sync_primary = p.integration_id.is_none() && root == project_root;
        let command = run_command(&root, project.run_cmd.as_deref(), p.variant.as_deref(), gradle_init.as_ref().map(|file| file.path()))?;
        let sdk_root = match android_sdk_root(ctx.tx()) {
            Ok(path) => Some(path),
            Err(error) if default_gradle_command => return Err(error),
            Err(_) => None,
        };
        ctx.tx().execute(
            "INSERT INTO device_runs(project_id,device,worktree,state,started_at) VALUES (?1,?2,?3,'building',?4)",
            params![project.id,p.device,root.display().to_string(),ctx.now],
        ).bus()?;
        let id = ctx.tx().last_insert_rowid();
        let run = get_run(ctx.tx(), id)?;
        let runtime = RunRuntime::new(id);
        let action = format!("device.run {} from {}", p.variant.as_deref().unwrap_or("debug"), run_source(&root, &project.path));
        ctx.engine().device_runs.lock().unwrap().insert(id, runtime.clone());
        if let Err(error) = super::device_lease::acquire(ctx, crate::device_lease::Lease::new(&p.device, holder, crate::device_lease::Kind::Run(id), &action, None)) {
            ctx.engine().device_runs.lock().unwrap().remove(&id);
            return Err(error);
        }
        let parent = ctx.req_id;
        let device_serial = p.device;
        let variant = p.variant.unwrap_or_else(|| "debug".to_string());
        let adb_string = adb.display().to_string();
        ctx.set_project(project.id);
        ctx.emit("run.changed", serde_json::to_value(&run).bus()?);
        let request = RunWorkerRequest {
            root,
            command,
            device: device_serial,
            adb: adb_string,
            sdk_root,
            launch_after_build: default_gradle_command,
            variant,
            _gradle_init: gradle_init,
            sync_primary,
        };
        ctx.after_commit(move |engine| {
            std::thread::spawn(move || run_worker(engine, runtime, parent, request));
        });
        Ok(run)
    });

    e.register::<Build>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        let root = resolve_run_root(ctx, project.id, &project.path, p.worktree.as_deref(), p.integration_id)?;
        let variant = p.variant.unwrap_or_else(|| "release".to_string());
        let format = p.format.unwrap_or_else(|| "apk".to_string());
        let publish = p.publish.unwrap_or(false);
        let signing_profile = load_signing_profile(ctx.engine(), &project.path)?.filter(|profile| profile.enabled);
        let signing_init = signing_profile.as_ref().map(|_| signing_init_script()).transpose()?;
        // A release build is Gradle's alone: no adb, no device, and no `build_cmd` detour —
        // that one belongs to the integration verifier. Builds also skip the primary-checkout
        // sync a run does: a release ships exactly what is checked out (D152).
        let command = build_command(&root, &variant, &format, publish, signing_init.as_ref().map(|file| file.path()))?;
        let sdk_root = android_sdk_root(ctx.tx())?;
        ctx.tx().execute(
            "INSERT INTO device_runs(project_id,kind,device,worktree,state,variant,format,publish,started_at) VALUES (?1,'build','',?2,'building',?3,?4,?5,?6)",
            params![project.id, root.display().to_string(), variant, format, publish as i64, ctx.now],
        ).bus()?;
        let id = ctx.tx().last_insert_rowid();
        let run = get_run(ctx.tx(), id)?;
        let runtime = RunRuntime::new(id);
        ctx.engine().device_runs.lock().unwrap().insert(id, runtime.clone());
        let parent = ctx.req_id;
        ctx.set_project(project.id);
        ctx.emit("run.changed", serde_json::to_value(&run).bus()?);
        let request = BuildWorkerRequest { root, command, sdk_root, variant, format, signing_profile, _signing_init: signing_init };
        ctx.after_commit(move |engine| {
            std::thread::spawn(move || build_worker(engine, runtime, parent, request));
        });
        Ok(run)
    });

    e.register_unlocked::<SigningGet>(|ctx, p| {
        let project = ctx.read(|conn| get_project(conn, p.project_id))?;
        signing_profile_out(ctx.engine(), &project.path)
    });

    // Key generation and Secret Service can both wait on external processes. The transaction
    // only attributes the completed profile and emits its state change (D149, D157).
    e.register_staged::<SigningCreate, _>(
        |ctx, p| {
            let project = ctx.read(|conn| get_project(conn, p.project_id))?;
            create_signing_profile(ctx.engine(), &project.path, &p.key_alias, &p.password)
        },
        |ctx: &mut Ctx, p, profile| {
            get_project(ctx.tx(), p.project_id)?;
            ctx.set_project(p.project_id);
            ctx.emit("device.signing.changed", json!({ "project_id": p.project_id }));
            Ok(profile)
        },
    );

    e.register::<SigningSetEnabled>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        let profile = set_signing_enabled(ctx.engine(), &project.path, p.enabled)?;
        ctx.set_project(p.project_id);
        ctx.emit("device.signing.changed", json!({ "project_id": p.project_id }));
        Ok(profile)
    });

    e.register::<RunStop>(|ctx: &mut Ctx, p| {
        let run = get_run(ctx.tx(), p.run_id)?;
        if let Some(runtime) = ctx.engine().device_runs.lock().unwrap().remove(&p.run_id) {
            runtime.stop();
        }
        for lease in ctx.engine().device_leases.release_run(p.run_id) {
            ctx.emit(super::device_lease::RELEASED, lease.event());
        }
        if matches!(run.state.as_str(), "building" | "running") {
            ctx.tx()
                .execute(
                    "UPDATE device_runs SET state='stopped',finished_at=?1 WHERE id=?2",
                    params![ctx.now, p.run_id],
                )
                .bus()?;
            ctx.set_project(run.project_id);
            ctx.emit(
                "run.changed",
                serde_json::to_value(get_run(ctx.tx(), p.run_id)?).bus()?,
            );
        }
        Ok(Empty {})
    });

    e.register::<RunList>(|ctx, p| {
        get_project(ctx.tx(), p.project_id)?;
        let mut st = ctx
            .tx()
            .prepare_cached("SELECT * FROM device_runs WHERE project_id=?1 ORDER BY id DESC LIMIT 50")
            .bus()?;
        let runs = st
            .query_map([p.project_id], run_row)
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        Ok(RunListOut { runs })
    });

    e.register_unlocked::<AvdList>(|ctx, _| {
        let (emulator, adb) = ctx.read(|conn| {
            Ok((sdk_tool(conn, "device.emulator_path", "emulator")?, adb_path(conn).ok()))
        })?;
        Ok(AvdListOut { avds: list_avds_with(&emulator, adb.as_deref())? })
    });
    e.register_unlocked::<AvdCatalog>(|ctx, _| {
        let (sdk, avdmanager) = ctx.read(|conn| {
            Ok((android_sdk_root(conn)?, sdk_tool(conn, "device.avdmanager_path", "avdmanager").ok()))
        })?;
        let mut system_images = installed_system_images(&sdk)?;
        system_images.sort();
        system_images.reverse();
        let devices = avdmanager
            .and_then(|avdmanager| Command::new(avdmanager).args(["list", "device", "-c"]).output().ok())
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string).collect())
            .unwrap_or_default();
        Ok(AvdCatalogOut { system_images, devices })
    });
    e.register::<AvdCreate>(|ctx: &mut Ctx, p| {
        let name = valid_avd_name(&p.name)?;
        if !p.package.starts_with("system-images;") {
            return Err(BusError::invalid("avd.package", "AVD package must be an installed system image"));
        }
        let avdmanager = sdk_tool(ctx.tx(), "device.avdmanager_path", "avdmanager")?;
        let mut command = Command::new(avdmanager);
        command.args(["create", "avd", "--name", &name, "--package", &p.package]);
        if let Some(device) = p.device.as_deref().filter(|value| !value.trim().is_empty()) {
            command.args(["--device", device]);
        }
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
            .map_err(|error| BusError::unavailable("avd.create_failed", error.to_string()))?;
        if let Some(stdin) = child.stdin.as_mut() { let _ = stdin.write_all(b"no\n"); }
        let output = child.wait_with_output().map_err(|error| BusError::unavailable("avd.create_failed", error.to_string()))?;
        if !output.status.success() {
            return Err(BusError::unavailable("avd.create_failed", String::from_utf8_lossy(&output.stderr).trim().to_string()));
        }
        let avd = list_avds(ctx.tx())?.into_iter().find(|avd| avd.name == name)
            .unwrap_or(Avd { name, device: p.device, package: Some(p.package), path: None, running_serial: None });
        ctx.emit("avd.changed", serde_json::to_value(&avd).bus()?);
        Ok(avd)
    });
    e.register::<AvdBoot>(|ctx: &mut Ctx, p| {
        let name = valid_avd_name(&p.name)?;
        if !list_avds(ctx.tx())?.iter().any(|avd| avd.name == name) {
            return Err(BusError::not_found("avd.not_found", format!("no AVD named {name}")));
        }
        let emulator = sdk_tool(ctx.tx(), "device.emulator_path", "emulator")?;
        let cold = p.cold.unwrap_or(false);
        let event_name = name.clone();
        ctx.emit("avd.changed", json!({"name":name,"state":"booting"}));
        ctx.after_commit(move |engine| {
            let mut command = Command::new(emulator);
            command.arg(format!("@{event_name}"));
            if cold { command.arg("-no-snapshot-load"); }
            command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            let state = if command.spawn().is_ok() { "started" } else { "failed" };
            engine.emit_system("avd.changed", json!({"name":event_name,"state":state}));
        });
        Ok(Empty {})
    });
}

fn device_watch_worker(engine: Arc<Engine>, runtime: Arc<DeviceWatchRuntime>, adb: PathBuf) {
    if runtime.stopped() { return; }
    let child = Command::new(adb).args(["track-devices", "-l"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else { clear_device_watch(&engine, &runtime); return; };
    let Some(stdout) = child.stdout.take() else { let _ = child.kill(); clear_device_watch(&engine, &runtime); return; };
    if !runtime.install(child) { return; }
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    while !runtime.stopped() {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => engine.emit_system("device.changed", json!({"reason":"system"})),
        }
    }
    runtime.finish();
    clear_device_watch(&engine, &runtime);
}

fn clear_device_watch(engine: &Engine, runtime: &Arc<DeviceWatchRuntime>) {
    let mut current = engine.device_watch.lock().unwrap();
    if current.runtime.as_ref().is_some_and(|value| Arc::ptr_eq(value, runtime)) { current.runtime.take(); }
}

fn valid_avd_name(value: &str) -> Result<String, BusError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 80 || !value.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')) {
        return Err(BusError::invalid("avd.name", "AVD name must use 1..=80 letters, digits, dots, dashes, or underscores"));
    }
    Ok(value.to_string())
}

fn sdk_tool(conn: &rusqlite::Connection, setting: &str, name: &str) -> Result<PathBuf, BusError> {
    let configured: Option<String> = conn.query_row("SELECT value FROM settings WHERE path=?1", [setting], |row| row.get(0)).optional().bus()?;
    if let Some(value) = configured.and_then(|value| serde_json::from_str::<String>(&value).ok()).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        if path.is_file() { return Ok(path); }
        return Err(BusError::unavailable("avd.tool_missing", format!("configured {name} does not exist: {}", path.display())));
    }
    if let Ok(path) = which::which(name) { return Ok(path); }
    if let Ok(sdk) = android_sdk_root(conn) {
        let direct = match name {
            "emulator" => Some(sdk.join("emulator/emulator")),
            "avdmanager" => find_avdmanager(&sdk),
            _ => None,
        };
        if let Some(path) = direct.filter(|path| path.is_file()) { return Ok(path); }
    }
    Err(BusError::unavailable("avd.tool_missing", format!("{name} is not installed or could not be detected")))
}

fn android_sdk_root(conn: &rusqlite::Connection) -> Result<PathBuf, BusError> {
    let configured: Option<String> = conn.query_row("SELECT value FROM settings WHERE path='device.sdk_path'", [], |row| row.get(0)).optional().bus()?;
    if let Some(value) = configured.and_then(|value| serde_json::from_str::<String>(&value).ok()).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        if path.is_dir() { return Ok(path); }
    }
    for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Some(path) = std::env::var_os(key).map(PathBuf::from).filter(|path| path.is_dir()) { return Ok(path); }
    }
    if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        for path in [home.join("Android/Sdk"), home.join("Android/sdk")] {
            if path.is_dir() { return Ok(path); }
        }
    }
    if let Ok(adb) = adb_path(conn) {
        if adb.parent().and_then(Path::file_name).is_some_and(|name| name == "platform-tools") {
            if let Some(root) = adb.parent().and_then(Path::parent).filter(|path| path.is_dir()) { return Ok(root.to_path_buf()); }
        }
    }
    Err(BusError::unavailable("avd.sdk_missing", "Android SDK root could not be detected").with_hint("set the Android SDK path in Settings"))
}

fn find_avdmanager(sdk: &Path) -> Option<PathBuf> {
    let latest = sdk.join("cmdline-tools/latest/bin/avdmanager");
    if latest.is_file() { return Some(latest); }
    let mut candidates = std::fs::read_dir(sdk.join("cmdline-tools")).ok()?.flatten()
        .map(|entry| entry.path().join("bin/avdmanager")).filter(|path| path.is_file()).collect::<Vec<_>>();
    candidates.sort();
    candidates.pop().or_else(|| {
        let legacy = sdk.join("tools/bin/avdmanager");
        legacy.is_file().then_some(legacy)
    })
}

fn installed_system_images(sdk: &Path) -> Result<Vec<String>, BusError> {
    let root = sdk.join("system-images");
    if !root.is_dir() { return Ok(Vec::new()); }
    let mut images = Vec::new();
    for api in std::fs::read_dir(&root).map_err(|error| BusError::unavailable("avd.catalog_failed", error.to_string()))? {
        let api = api.map_err(|error| BusError::unavailable("avd.catalog_failed", error.to_string()))?;
        if !api.path().is_dir() { continue; }
        for variant in std::fs::read_dir(api.path()).into_iter().flatten().flatten() {
            if !variant.path().is_dir() { continue; }
            for arch in std::fs::read_dir(variant.path()).into_iter().flatten().flatten() {
                if arch.path().is_dir() {
                    images.push(format!("system-images;{};{};{}", api.file_name().to_string_lossy(), variant.file_name().to_string_lossy(), arch.file_name().to_string_lossy()));
                }
            }
        }
    }
    Ok(images)
}

fn list_avds(conn: &rusqlite::Connection) -> Result<Vec<Avd>, BusError> {
    let emulator = sdk_tool(conn, "device.emulator_path", "emulator")?;
    let adb = adb_path(conn).ok();
    list_avds_with(&emulator, adb.as_deref())
}

/// The subprocess half of [`list_avds`], with both tool paths already read out of the store so
/// the emulator and adb calls happen with nothing locked (D144).
fn list_avds_with(emulator: &Path, adb: Option<&Path>) -> Result<Vec<Avd>, BusError> {
    let output = Command::new(emulator).arg("-list-avds").output()
        .map_err(|error| BusError::unavailable("avd.list_failed", error.to_string()))?;
    if !output.status.success() {
        return Err(BusError::unavailable("avd.list_failed", String::from_utf8_lossy(&output.stderr).trim().to_string()));
    }
    let running = adb.map(running_avds_with).transpose().ok().flatten().unwrap_or_default();
    let base = std::env::var_os("ANDROID_AVD_HOME").map(PathBuf::from).or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".android/avd")));
    let mut avds = Vec::new();
    for name in String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|line| !line.is_empty()) {
        let ini = base.as_ref().map(|base| base.join(format!("{name}.ini")));
        let path = ini.as_ref().and_then(|ini| read_property(ini, "path")).map(PathBuf::from)
            .or_else(|| base.as_ref().map(|base| base.join(format!("{name}.avd"))));
        let config = path.as_ref().map(|path| path.join("config.ini"));
        avds.push(Avd {
            name: name.to_string(),
            device: config.as_ref().and_then(|path| read_property(path, "hw.device.name")),
            package: config.as_ref().and_then(|path| read_property(path, "image.sysdir.1")).map(|value| image_package(&value)),
            path: path.map(|path| path.display().to_string()),
            running_serial: running.get(name).cloned(),
        });
    }
    Ok(avds)
}

fn running_avds_with(adb: &Path) -> Result<std::collections::HashMap<String, String>, BusError> {
    let mut running = std::collections::HashMap::new();
    for device in list_with_adb(adb)?.into_iter().filter(|device| device.kind == DeviceKind::Avd && device.state == "device") {
        if let Ok(output) = Command::new(adb).args(["-s", &device.serial, "emu", "avd", "name"]).output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let name = stdout.lines().map(str::trim).find(|line| !line.is_empty() && *line != "OK");
            if let Some(name) = name { running.insert(name.to_string(), device.serial); }
        }
    }
    Ok(running)
}

fn read_property(path: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|line| line.strip_prefix(&format!("{key}=")).map(str::trim).map(str::to_string))
}

fn image_package(value: &str) -> String {
    let normalized = value.trim().trim_matches('/').replace('\\', "/");
    if let Some(rest) = normalized.split("system-images/").nth(1) {
        format!("system-images;{}", rest.trim_matches('/').replace('/', ";"))
    } else { normalized.replace('/', ";") }
}

pub fn mirror_by_id(engine: &Engine, id: Id) -> Result<Arc<MirrorRuntime>, BusError> {
    engine
        .mirrors
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| BusError::not_found("device.mirror_not_found", format!("no mirror {id}")))
}

pub fn run_by_id(engine: &Engine, id: Id) -> Result<Arc<RunRuntime>, BusError> {
    engine
        .device_runs
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            BusError::not_found(
                "device.run_not_live",
                format!("run {id} has no live stream"),
            )
        })
}

fn adb_path(conn: &rusqlite::Connection) -> Result<PathBuf, BusError> {
    let configured: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE path='device.adb_path'",
            [],
            |row| row.get(0),
        )
        .optional()
        .bus()?;
    if let Some(value) = configured.and_then(|value| serde_json::from_str::<String>(&value).ok()) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(BusError::unavailable(
            "device.adb_missing",
            format!("configured adb does not exist: {}", path.display()),
        ));
    }
    which::which("adb").map_err(|_| {
        BusError::unavailable("device.adb_missing", "adb is not installed or not on PATH")
    })
}

/// `adb devices` starts the adb server when it is not already up, which can take seconds. This
/// runs inside the request transaction, so an unbounded wait would freeze every other bus op —
/// keystrokes included (D144). Bound it and refuse instead.
const ADB_LIST_TIMEOUT: Duration = Duration::from_secs(2);

fn list_with_adb(adb: &Path) -> Result<Vec<Device>, BusError> {
    let mut command = Command::new(adb);
    command.args(["devices", "-l"]);
    let output = crate::proc::output_with_timeout(&mut command, ADB_LIST_TIMEOUT)
        .map_err(|e| BusError::unavailable("device.adb_failed", e.to_string()))?
        .ok_or_else(|| {
            BusError::unavailable(
                "device.adb_timeout",
                format!("adb did not answer within {}s", ADB_LIST_TIMEOUT.as_secs()),
            )
            .with_hint("the adb server may be starting; try again in a moment")
        })?;
    if !output.status.success() {
        return Err(BusError::unavailable(
            "device.adb_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let mut devices = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines().skip(1) {
        let mut fields = line.split_whitespace();
        let Some(serial) = fields.next() else {
            continue;
        };
        let Some(state) = fields.next() else { continue };
        if serial.starts_with('*') {
            continue;
        }
        let extras: Vec<_> = fields.collect();
        let model = extras
            .iter()
            .find_map(|field| field.strip_prefix("model:"))
            .unwrap_or(serial)
            .replace('_', " ");
        devices.push(Device {
            serial: serial.to_string(),
            model,
            kind: if serial.starts_with("emulator-") {
                DeviceKind::Avd
            } else {
                DeviceKind::Usb
            },
            state: state.to_string(),
            lease: None,
        });
    }
    Ok(devices)
}

fn require_device(adb: &Path, serial: &str) -> Result<Device, BusError> {
    list_with_adb(adb)?
        .into_iter()
        .find(|device| device.serial == serial)
        .ok_or_else(|| {
            BusError::unavailable("device.none", format!("device {serial} is not connected"))
        })
}

const ADB_SIZE_TIMEOUT: Duration = Duration::from_secs(5);

fn device_size(adb: &Path, serial: &str) -> Result<(u32, u32), BusError> {
    let mut command = Command::new(adb);
    command.args(["-s", serial, "shell", "wm", "size"]);
    let output = crate::proc::output_with_timeout(&mut command, ADB_SIZE_TIMEOUT)
        .map_err(|e| BusError::unavailable("device.size_failed", e.to_string()))?
        .ok_or_else(|| {
            BusError::unavailable(
                "device.size_timeout",
                format!("`adb shell wm size` did not answer within {}s", ADB_SIZE_TIMEOUT.as_secs()),
            )
            .with_hint("the device may be locked up or still booting; replug it and try again")
        })?;
    if !output.status.success() {
        return Err(BusError::unavailable(
            "device.size_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .rev()
        .find_map(|line| line.split_whitespace().last())
        .and_then(|size| size.split_once('x'))
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .ok_or_else(|| {
            BusError::unavailable(
                "device.size_failed",
                format!("unrecognized wm size output: {}", text.trim()),
            )
        })
}

fn new_scid(id: Id) -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    ((nanos ^ std::process::id().rotate_left(16) ^ (id as u32).rotate_left(7)) & 0x7fff_ffff).max(1)
}

/// One short adb step of a mirror's setup or teardown, under a deadline. The worker is off the
/// bus, but a push that never returns would still leave the window on "Connecting" forever.
fn mirror_adb(runtime: &MirrorRuntime, args: Vec<String>, timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(&runtime.adb);
    command.args(&args);
    let output = crate::proc::output_with_timeout(&mut command, timeout)
        .map_err(|error| format!("adb: {error}"))?
        .ok_or_else(|| format!("adb {} did not finish within {}s", args.join(" "), timeout.as_secs()))?;
    if !output.status.success() {
        return Err(format!(
            "adb {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn connect_mirror_video(port: u16, deadline: Instant) -> Result<TcpStream, String> {
    loop {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            let mut dummy = [0u8; 1];
            if stream.read_exact(&mut dummy).is_ok() {
                return Ok(stream);
            }
        }
        if Instant::now() >= deadline {
            return Err("mirror server did not come up".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn drain_server_lines(pipe: Option<impl Read + Send + 'static>, ring: Arc<Mutex<Vec<String>>>) {
    let Some(pipe) = pipe else { return };
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            let mut lines = ring.lock().unwrap();
            lines.push(line);
            let overflow = lines.len().saturating_sub(30);
            if overflow > 0 {
                lines.drain(..overflow);
            }
        }
    });
}

const MIRROR_PUSH_TIMEOUT: Duration = Duration::from_secs(60);
const MIRROR_FORWARD_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the server gets between the handshake and its first session packet. An encoder
/// that never starts is a failure to report, not a window to leave on "Connecting".
const MIRROR_FIRST_UNIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Report a mirror that ended without being asked to: the `mirror.changed` event for every
/// listener, and the runtime's terminal status, which is what the socket door forwards to
/// the window that owns it — the window does not have to be subscribed to anything.
fn mirror_failure(engine: &Engine, runtime: &MirrorRuntime, state: MirrorState, code: &str, message: String) {
    if !runtime.finish(state, Some(code.to_string()), Some(message.clone())) {
        return;
    }
    let state = if state == MirrorState::Lost { "lost" } else { "failed" };
    engine.emit_system(
        "mirror.changed",
        json!({"mirror_id":runtime.id,"device":runtime.device,"state":state,"code":code,"message":message}),
    );
}

/// Whether adb still sees the device. Asked once, after the stream broke, to tell "unplugged"
/// from "the server died" — the two need different words and a different next step.
fn device_present(runtime: &MirrorRuntime) -> bool {
    mirror_adb(runtime, vec!["-s".into(), runtime.device.clone(), "get-state".into()], Duration::from_secs(3))
        .is_ok_and(|state| state.trim() == "device")
}

/// The transport half of a mirror: push, forward, boot the server, pump the stream, and on the
/// way out report how it ended. Public so the integration tests can drive it against a fake
/// adb and a fake server; the engine itself only ever starts it from `device.mirror.start`.
#[doc(hidden)]
pub fn mirror_worker(engine: Arc<Engine>, runtime: Arc<MirrorRuntime>) {
    let fail = |code: &str, message: String| {
        mirror_failure(&engine, &runtime, MirrorState::Failed, code, message);
        engine.mirrors.lock().unwrap().remove(&runtime.id);
    };
    let Some(server_jar) = mirror_server_path() else {
        fail("device.mirror_server_missing", "the bundled scrcpy server is missing".into());
        return;
    };

    if let Err(error) = mirror_adb(&runtime, mirror::push_args(&runtime.device, &server_jar.to_string_lossy()), MIRROR_PUSH_TIMEOUT) {
        fail("device.mirror_push_failed", error);
        return;
    }
    let port = match mirror_adb(&runtime, mirror::forward_args(&runtime.device, runtime.scid), MIRROR_FORWARD_TIMEOUT)
        .and_then(|value| value.trim().parse::<u16>().map_err(|_| format!("adb forward returned no port ({value:?})")))
    {
        Ok(port) => port,
        Err(error) => {
            fail("device.mirror_forward_failed", error);
            return;
        }
    };
    if runtime.stopped() {
        let _ = mirror_adb(&runtime, mirror::forward_remove_args(&runtime.device, runtime.scid), MIRROR_FORWARD_TIMEOUT);
        engine.mirrors.lock().unwrap().remove(&runtime.id);
        return;
    }

    let mut options = mirror::MirrorOptions::for_capture(runtime.max_size);
    options.video_bit_rate = runtime.bitrate;
    let mut command = Command::new(&runtime.adb);
    command
        .args(mirror::server_shell_args_with(&runtime.device, runtime.scid, &options))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut server = match command.spawn() {
        Ok(server) => server,
        Err(error) => {
            let _ = mirror_adb(&runtime, mirror::forward_remove_args(&runtime.device, runtime.scid), MIRROR_FORWARD_TIMEOUT);
            fail("device.mirror_spawn_failed", error.to_string());
            return;
        }
    };
    let logs: Arc<Mutex<Vec<String>>> = Arc::default();
    drain_server_lines(server.stdout.take(), logs.clone());
    drain_server_lines(server.stderr.take(), logs.clone());
    runtime.set_child(server);

    let result = (|| -> Result<(), String> {
        let mut video = connect_mirror_video(port, Instant::now() + Duration::from_secs(10))?;
        let control = TcpStream::connect(("127.0.0.1", port))
            .map_err(|error| format!("mirror control socket: {error}"))?;
        video.set_read_timeout(Some(Duration::from_secs(5))).map_err(|error| error.to_string())?;
        let mut name = [0u8; 64];
        video.read_exact(&mut name).map_err(|error| format!("mirror handshake (device name): {error}"))?;
        let mut codec = [0u8; 4];
        video.read_exact(&mut codec).map_err(|error| format!("mirror handshake (codec id): {error}"))?;
        if mirror::parse_codec_id(&codec) != mirror::CODEC_ID_H264 {
            return Err("mirror device did not provide H.264 video".into());
        }
        let _ = video.set_nodelay(true);
        let _ = control.set_nodelay(true);
        let keepalive = video.try_clone().map_err(|error| error.to_string())?;
        let mut drain = control.try_clone().map_err(|error| error.to_string())?;
        runtime.set_video(keepalive);
        runtime.install_control(control).map_err(|error| format!("mirror control socket: {error}"))?;
        std::thread::spawn(move || {
            let mut bytes = [0u8; 4096];
            while drain.read(&mut bytes).is_ok_and(|count| count > 0) {}
        });
        let device_name = String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_string();
        // "Running" waits for the first unit — the session packet that carries the real
        // picture size — so the window and tap/swipe never work from the `wm size` estimate.
        video.set_read_timeout(Some(MIRROR_FIRST_UNIT_TIMEOUT)).map_err(|error| error.to_string())?;
        let mut announced = false;
        let announce = |announced: &mut bool| {
            if *announced {
                return;
            }
            *announced = true;
            runtime.set_running(device_name.clone());
            let (width, height) = runtime.size();
            engine.emit_system(
                "mirror.changed",
                json!({"mirror_id":runtime.id,"device":runtime.device,"name":device_name,"state":"running","width":width,"height":height}),
            );
        };
        while !runtime.stopped() {
            let mut header = [0u8; 12];
            video.read_exact(&mut header).map_err(|error| {
                if announced { format!("mirror stream ended: {error}") } else { format!("mirror server sent no video: {error}") }
            })?;
            if !announced {
                // The first-unit deadline covers the handshake only; once frames flow, a
                // static screen can legitimately send nothing for minutes.
                video.set_read_timeout(None).map_err(|error| error.to_string())?;
            }
            match mirror::parse_stream_unit(&header) {
                mirror::StreamUnit::Session { width, height } => {
                    runtime.set_size(width, height);
                    if announced {
                        engine.emit_system(
                            "mirror.changed",
                            json!({"mirror_id":runtime.id,"device":runtime.device,"state":"running","width":width,"height":height}),
                        );
                    }
                    announce(&mut announced);
                }
                mirror::StreamUnit::Media { config, key, len, .. } => {
                    announce(&mut announced);
                    if len == 0 || len > 16 * 1024 * 1024 {
                        return Err("mirror stream desynced".into());
                    }
                    let mut packet = vec![0u8; 1 + len as usize];
                    packet[0] = u8::from(config) | (u8::from(key) << 1);
                    video.read_exact(&mut packet[1..]).map_err(|error| format!("mirror stream ended: {error}"))?;
                    runtime.push(packet);
                }
            }
        }
        Ok(())
    })();

    let requested_stop = runtime.stopped();
    runtime.stop();
    if let Some(mut child) = runtime.take_child() {
        let _ = child.wait();
    }
    let _ = mirror_adb(&runtime, mirror::forward_remove_args(&runtime.device, runtime.scid), MIRROR_FORWARD_TIMEOUT);
    match result {
        Err(error) if !requested_stop => {
            if device_present(&runtime) {
                let tail = logs.lock().unwrap().iter().rev().take(5).cloned().collect::<Vec<_>>();
                let message = if tail.is_empty() { error } else { format!("{error}\n{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n")) };
                mirror_failure(&engine, &runtime, MirrorState::Failed, "device.mirror_stream_failed", message);
            } else {
                mirror_failure(
                    &engine,
                    &runtime,
                    MirrorState::Lost,
                    "device.mirror_device_lost",
                    format!("{} is no longer connected", runtime.device),
                );
            }
        }
        _ => {
            runtime.finish(MirrorState::Stopped, None, None);
        }
    }
    engine.mirrors.lock().unwrap().remove(&runtime.id);
}

fn event_coordinate(event: &Value, name: &str, limit: u32) -> Result<i32, BusError> {
    let value = event.get(name).and_then(Value::as_i64).ok_or_else(|| {
        BusError::invalid("device.input", format!("event.{name} must be an integer"))
    })?;
    if !(0..limit as i64).contains(&value) {
        return Err(BusError::invalid("device.input", format!("event.{name} is outside the mirrored display ({limit} px)")));
    }
    Ok(value as i32)
}

/// Encode and write one `device.mirror.input` event. Pure memory and one socket write — no
/// store access — which is what lets the engine answer it on the fast path. `tap`/`swipe`
/// coordinates are in the stream's *current* size, rotation included.
pub fn send_input(runtime: &MirrorRuntime, event: &Value) -> Result<(), BusError> {
    let kind = event.get("type").and_then(Value::as_str)
        .ok_or_else(|| BusError::invalid("device.input", "event.type is required"))?;
    let (width, height) = runtime.size();
    let (w, h) = (width as u16, height as u16);
    let message = match kind {
        "tap" => {
            let x = event_coordinate(event, "x", width)?;
            let y = event_coordinate(event, "y", height)?;
            let mut bytes = mirror::touch(mirror::ACTION_DOWN, x, y, w, h, 1.0).to_vec();
            bytes.extend_from_slice(&mirror::touch(mirror::ACTION_UP, x, y, w, h, 0.0));
            bytes
        }
        "swipe" => {
            let x1 = event_coordinate(event, "x1", width)?;
            let y1 = event_coordinate(event, "y1", height)?;
            let x2 = event_coordinate(event, "x2", width)?;
            let y2 = event_coordinate(event, "y2", height)?;
            let mut bytes = mirror::touch(mirror::ACTION_DOWN, x1, y1, w, h, 1.0).to_vec();
            bytes.extend_from_slice(&mirror::touch(mirror::ACTION_MOVE, x2, y2, w, h, 1.0));
            bytes.extend_from_slice(&mirror::touch(mirror::ACTION_UP, x2, y2, w, h, 0.0));
            bytes
        }
        _ => {
            let input = serde_json::from_value::<mirror::InputMsg>(event.clone())
                .map_err(|error| BusError::invalid("device.input", error.to_string()))?;
            mirror::encode_input(&input)
        }
    };
    runtime.send_control(message)
        .map_err(|error| BusError::unavailable("device.input_failed", error.to_string()))
}

/// The lease holder for a run an agent session asked for itself.
fn sessions_holder(ctx: &Ctx, id: Id) -> Result<crate::device_lease::Holder, BusError> {
    let row = crate::sessions::by_id(ctx.tx(), id)?.ok_or_else(|| BusError::internal("bound session vanished"))?;
    Ok(crate::device_lease::Holder::Session { id, name: row.session.name })
}

/// `relay/brisk-otter` for a pooled checkout, `the primary checkout` otherwise: what a refused
/// caller needs to recognize whose build is on the phone.
fn run_source(root: &Path, project_path: &str) -> String {
    let primary = std::fs::canonicalize(project_path).ok();
    if primary.as_deref() == Some(root) {
        return "the primary checkout".into();
    }
    gix::open(root).ok()
        .and_then(|repo| repo.head_name().ok().flatten().map(|name| name.shorten().to_string()))
        .unwrap_or_else(|| root.display().to_string())
}

fn resolve_run_root(
    ctx: &Ctx,
    project_id: Id,
    project_path: &str,
    requested: Option<&str>,
    integration_id: Option<Id>,
) -> Result<PathBuf, BusError> {
    let selected = if let Some(integration_id) = integration_id {
        let row: Option<(Id, Option<String>, String)> = ctx
            .tx()
            .query_row(
                "SELECT project_id,worktree,state FROM integrations WHERE id=?1",
                [integration_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .bus()?;
        let (owner, path, state) = row.ok_or_else(|| {
            BusError::not_found(
                "integration.not_found",
                format!("no integration {integration_id}"),
            )
        })?;
        if owner != project_id {
            return Err(BusError::invalid(
                "device.integration_project",
                "integration belongs to another project",
            ));
        }
        if state != "passed" {
            return Err(BusError::conflict(
                "device.integration_not_ready",
                format!("integration {integration_id} is {state}"),
            ));
        }
        PathBuf::from(path.ok_or_else(|| {
            BusError::conflict(
                "device.integration_not_ready",
                "integration has no worktree",
            )
        })?)
    } else {
        requested
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(project_path))
    };
    let selected = std::fs::canonicalize(&selected)
        .map_err(|e| BusError::invalid("device.worktree", e.to_string()))?;
    // Membership only. Listing the worktrees with a dirty check ran a `git status` per checkout
    // before a build had even looked for its Gradle wrapper (PERF §1.4).
    if !worktree::contains(Path::new(project_path), &selected)
        .map_err(|e| BusError::unavailable("worktree.list_failed", e.to_string()))?
    {
        return Err(BusError::invalid(
            "device.worktree",
            "worktree does not belong to project",
        ));
    }
    Ok(selected)
}

fn run_command(
    root: &Path,
    configured: Option<&str>,
    variant: Option<&str>,
    init_script: Option<&Path>,
) -> Result<String, BusError> {
    if let Some(command) = configured.filter(|command| !command.trim().is_empty()) {
        return Ok(command.to_string());
    }
    let wrapper = gradle_wrapper(root).ok_or_else(|| {
        BusError::unavailable(
            "device.gradle_missing",
            "no run command or Gradle wrapper is configured",
        )
    })?;
    let task = format!("install{}", gradle_variant(variant.unwrap_or("debug"))?);
    let init = init_script.map(|path| format!(" --init-script '{}'", path.display())).unwrap_or_default();
    Ok(format!("{wrapper}{init} {task}"))
}

/// `./gradlew` as the run command spells it, from the project root or a `forge-android`
/// subproject; `None` when the checkout has no wrapper at all.
fn gradle_wrapper(root: &Path) -> Option<String> {
    let wrapper = [root.join("gradlew"), root.join("forge-android/gradlew")]
        .into_iter()
        .find(|path| path.is_file())?;
    Some(format!("./{}", wrapper.strip_prefix(root).unwrap_or(&wrapper).display()))
}

/// `release` → `Release`: the variant as Gradle spells it inside a task name. Rejects
/// anything that would not survive being pasted into a shell command.
fn gradle_variant(variant: &str) -> Result<String, BusError> {
    if variant.is_empty() || !variant.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(BusError::invalid(
            "device.variant",
            "variant may contain only letters, numbers, and underscores",
        ));
    }
    let mut chars = variant.chars();
    Ok(format!("{}{}", chars.next().unwrap().to_ascii_uppercase(), chars.as_str()))
}

/// The Gradle task a release build runs. `publish` hands the artifact to Gradle Play
/// Publisher, which owns the Play credentials. An optional Relay signing init script changes
/// only this invocation and never writes into the selected Android checkout (D152, D157).
fn build_command(
    root: &Path,
    variant: &str,
    format: &str,
    publish: bool,
    signing_init: Option<&Path>,
) -> Result<String, BusError> {
    let wrapper = gradle_wrapper(root).ok_or_else(|| {
        BusError::unavailable(
            "device.gradle_missing",
            "no Gradle wrapper in the selected worktree",
        )
    })?;
    let variant = gradle_variant(variant)?;
    let task = match (publish, format) {
        (false, "apk") => format!("assemble{variant}"),
        (false, "bundle") => format!("bundle{variant}"),
        (true, "apk") => format!("publish{variant}Apk"),
        (true, "bundle") => format!("publish{variant}Bundle"),
        _ => {
            return Err(BusError::invalid(
                "device.format",
                "format must be \"apk\" or \"bundle\"",
            ))
        }
    };
    let signing = signing_init
        .map(|path| format!(" --init-script '{}'", path.display()))
        .unwrap_or_default();
    Ok(format!("{wrapper} {task}{signing}"))
}

const GRADLE_RELAY_SIGNING_INIT: &str = r#"gradle.beforeProject { target ->
    target.plugins.withId("com.android.application") {
        def androidComponents = target.extensions.getByName("androidComponents")
        androidComponents.finalizeDsl { android ->
            def relaySigning = android.signingConfigs.findByName("relayRelease")
            if (relaySigning == null) relaySigning = android.signingConfigs.create("relayRelease")
            relaySigning.storeFile = target.file(System.getenv("RELAY_SIGNING_STORE_FILE"))
            relaySigning.storePassword = System.getenv("RELAY_SIGNING_STORE_PASSWORD")
            relaySigning.keyAlias = System.getenv("RELAY_SIGNING_KEY_ALIAS")
            relaySigning.keyPassword = System.getenv("RELAY_SIGNING_KEY_PASSWORD")
            def release = android.buildTypes.findByName("release")
            if (release == null) throw new GradleException("Relay signing needs an Android release build type")
            release.signingConfig = relaySigning
        }
    }
}
"#;

fn signing_init_script() -> Result<tempfile::NamedTempFile, BusError> {
    let mut file = tempfile::Builder::new()
        .prefix("relay-signing-")
        .suffix(".gradle")
        .tempfile()
        .map_err(|error| BusError::unavailable("device.signing_init_failed", error.to_string()))?;
    file.write_all(GRADLE_RELAY_SIGNING_INIT.as_bytes())
        .map_err(|error| BusError::unavailable("device.signing_init_failed", error.to_string()))?;
    Ok(file)
}

const SIGNING_PROFILE_FILE: &str = "profile.json";
const SIGNING_KEYSTORE_FILE: &str = "release.p12";
const TEST_SIGNING_SECRET_FILE: &str = ".test-secret";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigningProfileDisk {
    project: String,
    key_alias: String,
    keystore: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

#[derive(Debug, Clone)]
struct SigningRuntimeProfile {
    enabled: bool,
    key_alias: String,
    keystore: PathBuf,
    secret_id: String,
    directory: PathBuf,
}

fn enabled_by_default() -> bool {
    true
}

fn validate_signing_alias(alias: &str) -> Result<(), BusError> {
    if alias.is_empty()
        || alias.len() > 64
        || !alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err(BusError::invalid(
            "device.signing_alias",
            "key alias must be 1-64 letters, numbers, dots, underscores, or hyphens",
        ));
    }
    Ok(())
}

fn validate_signing_input(alias: &str, password: &str) -> Result<(), BusError> {
    validate_signing_alias(alias)?;
    let password_len = password.chars().count();
    if !(6..=256).contains(&password_len) || password.contains(['\0', '\r', '\n']) {
        return Err(BusError::invalid(
            "device.signing_password",
            "password must be 6-256 characters with no line breaks",
        ));
    }
    Ok(())
}

fn signing_root(engine: &Engine) -> PathBuf {
    engine
        .store
        .path()
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.join("signing"))
        .unwrap_or_else(|| std::env::temp_dir().join(format!("relay-signing-test-{}", std::process::id())))
}

fn signing_project_key(project: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(project.as_bytes());
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn signing_directory(engine: &Engine, project: &str) -> PathBuf {
    signing_root(engine).join(signing_project_key(project))
}

fn load_signing_profile_disk(
    engine: &Engine,
    project: &str,
) -> Result<Option<(PathBuf, SigningProfileDisk)>, BusError> {
    let directory = signing_directory(engine, project);
    let profile_path = directory.join(SIGNING_PROFILE_FILE);
    if !profile_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&profile_path)
        .map_err(|error| BusError::unavailable("device.signing_profile_unreadable", error.to_string()))?;
    let profile: SigningProfileDisk = serde_json::from_str(&text)
        .map_err(|error| BusError::unavailable("device.signing_profile_invalid", error.to_string()))?;
    if profile.project != project || profile.keystore != SIGNING_KEYSTORE_FILE {
        return Err(BusError::unavailable(
            "device.signing_profile_invalid",
            "the saved signing profile does not match this project",
        ));
    }
    validate_signing_alias(&profile.key_alias)?;
    let keystore = directory.join(&profile.keystore);
    if !keystore.is_file() {
        return Err(BusError::unavailable(
            "device.signing_keystore_missing",
            format!("saved keystore is missing: {}", keystore.display()),
        ));
    }
    Ok(Some((directory, profile)))
}

fn load_signing_profile(engine: &Engine, project: &str) -> Result<Option<SigningRuntimeProfile>, BusError> {
    let Some((directory, profile)) = load_signing_profile_disk(engine, project)? else {
        return Ok(None);
    };
    Ok(Some(SigningRuntimeProfile {
        enabled: profile.enabled,
        key_alias: profile.key_alias,
        keystore: directory.join(&profile.keystore),
        secret_id: signing_project_key(project),
        directory,
    }))
}

fn signing_profile_out(engine: &Engine, project: &str) -> Result<SigningProfileOut, BusError> {
    let Some(profile) = load_signing_profile(engine, project)? else {
        return Ok(SigningProfileOut { configured: false, enabled: false, key_alias: None, keystore: None });
    };
    Ok(SigningProfileOut {
        configured: true,
        enabled: profile.enabled,
        key_alias: Some(profile.key_alias),
        keystore: Some(profile.keystore.display().to_string()),
    })
}

fn create_signing_profile(
    engine: &Engine,
    project: &str,
    alias: &str,
    password: &str,
) -> Result<SigningProfileOut, BusError> {
    validate_signing_input(alias, password)?;
    let root = signing_root(engine);
    fs::create_dir_all(&root)
        .map_err(|error| BusError::unavailable("device.signing_store_failed", error.to_string()))?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .map_err(|error| BusError::unavailable("device.signing_store_failed", error.to_string()))?;
    let directory = signing_directory(engine, project);
    fs::create_dir(&directory).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            BusError::conflict("device.signing_exists", "this project already has a Relay signing profile")
        } else {
            BusError::unavailable("device.signing_store_failed", error.to_string())
        }
    })?;

    let keystore = directory.join(SIGNING_KEYSTORE_FILE);
    let result = (|| {
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| BusError::unavailable("device.signing_store_failed", error.to_string()))?;
        generate_signing_keystore(engine.instance, &keystore, alias, password)?;
        let profile = SigningProfileDisk {
            project: project.to_string(),
            key_alias: alias.to_string(),
            keystore: SIGNING_KEYSTORE_FILE.to_string(),
            enabled: true,
        };
        let bytes = serde_json::to_vec_pretty(&profile)
            .map_err(|error| BusError::internal(format!("serializing signing profile: {error}")))?;
        write_private_new(&directory.join(SIGNING_PROFILE_FILE), &bytes)
            .map_err(|error| BusError::unavailable("device.signing_store_failed", error.to_string()))?;
        store_signing_secret(engine.instance, &directory, &signing_project_key(project), project, password)?;
        Ok(SigningProfileOut {
            configured: true,
            enabled: true,
            key_alias: Some(alias.to_string()),
            keystore: Some(keystore.display().to_string()),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    result
}

fn set_signing_enabled(engine: &Engine, project: &str, enabled: bool) -> Result<SigningProfileOut, BusError> {
    let Some((directory, mut profile)) = load_signing_profile_disk(engine, project)? else {
        return Err(BusError::not_found(
            "device.signing_missing",
            "this project has no Relay signing profile",
        ));
    };
    profile.enabled = enabled;
    let bytes = serde_json::to_vec_pretty(&profile)
        .map_err(|error| BusError::internal(format!("serializing signing profile: {error}")))?;
    write_private_replace(&directory.join(SIGNING_PROFILE_FILE), &bytes)
        .map_err(|error| BusError::unavailable("device.signing_store_failed", error.to_string()))?;
    signing_profile_out(engine, project)
}

fn write_private_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn write_private_replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = tempfile::Builder::new().prefix(".relay-signing-profile-").tempfile_in(
        path.parent().ok_or_else(|| std::io::Error::other("signing profile has no parent directory"))?,
    )?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn keytool_path() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("bin/keytool"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("keytool"))
}

fn generate_signing_keystore(instance: Instance, path: &Path, alias: &str, password: &str) -> Result<(), BusError> {
    if instance == Instance::Test {
        return write_private_new(path, b"Relay test keystore")
            .map_err(|error| BusError::unavailable("device.keytool_failed", error.to_string()));
    }
    let mut command = Command::new(keytool_path());
    command
        .args([
            "-genkeypair",
            "-keystore",
            path.to_string_lossy().as_ref(),
            "-storetype",
            "PKCS12",
            "-alias",
            alias,
            "-keyalg",
            "RSA",
            "-keysize",
            "4096",
            "-validity",
            "10000",
            "-dname",
            "CN=Relay Android Release",
            "-storepass:env",
            "RELAY_SIGNING_PASSWORD",
            "-keypass:env",
            "RELAY_SIGNING_PASSWORD",
        ])
        .env("RELAY_SIGNING_PASSWORD", password);
    let output = crate::proc::output_with_timeout(&mut command, Duration::from_secs(90))
        .map_err(|error| BusError::unavailable("device.keytool_missing", error.to_string()))?
        .ok_or_else(|| BusError::unavailable("device.keytool_timeout", "keytool did not finish within 90 seconds"))?;
    if !output.status.success() {
        return Err(BusError::conflict(
            "device.keytool_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| BusError::unavailable("device.signing_store_failed", error.to_string()))
}

fn store_signing_secret(
    instance: Instance,
    directory: &Path,
    secret_id: &str,
    project: &str,
    password: &str,
) -> Result<(), BusError> {
    if instance == Instance::Test {
        return write_private_new(&directory.join(TEST_SIGNING_SECRET_FILE), password.as_bytes())
            .map_err(|error| BusError::unavailable("device.signing_secret_failed", error.to_string()));
    }
    const STORE: &str = r#"printf %s "$RELAY_SIGNING_PASSWORD" | secret-tool store --label="$RELAY_SIGNING_LABEL" application Relay purpose android-signing project "$RELAY_SIGNING_PROJECT""#;
    let label = format!(
        "Relay Android signing: {}",
        Path::new(project).file_name().and_then(|name| name.to_str()).unwrap_or(project)
    );
    let mut command = Command::new("sh");
    command
        .args(["-c", STORE])
        .env("RELAY_SIGNING_PASSWORD", password)
        .env("RELAY_SIGNING_LABEL", label)
        .env("RELAY_SIGNING_PROJECT", secret_id);
    let output = crate::proc::output_with_timeout(&mut command, Duration::from_secs(30))
        .map_err(|error| BusError::unavailable("device.secret_service_missing", error.to_string()))?
        .ok_or_else(|| BusError::unavailable("device.signing_secret_timeout", "Linux Secret Service did not answer within 30 seconds"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BusError::unavailable(
            "device.signing_secret_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )
        .with_hint("install libsecret and unlock your desktop keyring"))
    }
}

fn load_signing_secret(instance: Instance, profile: &SigningRuntimeProfile) -> Result<String, BusError> {
    if instance == Instance::Test {
        return fs::read_to_string(profile.directory.join(TEST_SIGNING_SECRET_FILE))
            .map_err(|error| BusError::unavailable("device.signing_secret_missing", error.to_string()));
    }
    let mut command = Command::new("secret-tool");
    command.args([
        "lookup",
        "application",
        "Relay",
        "purpose",
        "android-signing",
        "project",
        &profile.secret_id,
    ]);
    let output = crate::proc::output_with_timeout(&mut command, Duration::from_secs(15))
        .map_err(|error| BusError::unavailable("device.secret_service_missing", error.to_string()))?
        .ok_or_else(|| BusError::unavailable("device.signing_secret_timeout", "Linux Secret Service did not answer within 15 seconds"))?;
    let password = String::from_utf8_lossy(&output.stdout).trim_end_matches(['\r', '\n']).to_string();
    if !output.status.success() || password.is_empty() {
        return Err(BusError::unavailable(
            "device.signing_secret_missing",
            "the password for this Relay signing profile is not in Linux Secret Service",
        )
        .with_hint("unlock your desktop keyring, then rebuild"));
    }
    Ok(password)
}

const GRADLE_ALLOW_DOWNGRADE_INIT: &str = r#"allprojects {
    plugins.withId("com.android.application") {
        def android = extensions.findByName("android")
        if (android.hasProperty("installation")) {
            android.installation.installOptions.add("-d")
        } else if (android.hasProperty("adbOptions")) {
            android.adbOptions.installOptions.add("-d")
        }
    }
}
"#;

fn gradle_init_script() -> Result<tempfile::NamedTempFile, BusError> {
    let mut file = tempfile::Builder::new().prefix("relay-gradle-").suffix(".gradle").tempfile()
        .map_err(|error| BusError::unavailable("device.gradle_init_failed", error.to_string()))?;
    file.write_all(GRADLE_ALLOW_DOWNGRADE_INIT.as_bytes())
        .map_err(|error| BusError::unavailable("device.gradle_init_failed", error.to_string()))?;
    Ok(file)
}

#[derive(Debug)]
struct PrimarySyncError {
    code: &'static str,
    message: String,
}

impl PrimarySyncError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

fn git_capture(root: &Path, args: &[&str]) -> Result<String, PrimarySyncError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| PrimarySyncError::new("device.sync_failed", format!("could not run git: {error}")))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(PrimarySyncError::new(
            "device.sync_failed",
            if detail.is_empty() { format!("git {} failed", args.join(" ")) } else { detail },
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_optional(root: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_is_ancestor(root: &Path, ancestor: &str, descendant: &str) -> Result<bool, PrimarySyncError> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .status()
        .map_err(|error| PrimarySyncError::new("device.sync_failed", format!("could not compare revisions: {error}")))?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(PrimarySyncError::new(
            "device.sync_failed",
            "git could not compare the primary branch with its upstream",
        )),
    }
}

fn short_oid(oid: &str) -> &str {
    oid.get(..oid.len().min(8)).unwrap_or(oid)
}

fn sync_primary_checkout(root: &Path) -> Result<String, PrimarySyncError> {
    let Some(branch) = git_optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .filter(|branch| !branch.is_empty())
    else {
        return Ok("primary sync: detached HEAD has no upstream; using local commit".into());
    };
    let Some(upstream) = git_optional(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    )
    .filter(|upstream| !upstream.is_empty())
    else {
        return Ok(format!("primary sync: {branch} has no upstream; using local commit"));
    };

    let repo = gix::open(root)
        .map_err(|error| PrimarySyncError::new("device.sync_failed", format!("could not inspect primary checkout: {error}")))?;
    if worktree::is_dirty(&repo) {
        return Err(PrimarySyncError::new(
            "device.sync_dirty",
            "primary checkout has uncommitted changes; Relay will not fetch or overwrite it",
        ));
    }

    let remote = git_capture(root, &["config", "--get", &format!("branch.{branch}.remote")])?;
    if remote != "." {
        worktree::git_mutate(root, &["fetch", "--quiet", "--no-tags", &remote]).map_err(|error| {
            PrimarySyncError::new(
                "device.sync_fetch_failed",
                format!("could not refresh {upstream}: {error}"),
            )
        })?;
    }

    let head = git_capture(root, &["rev-parse", "HEAD"])?;
    let upstream_head = git_capture(root, &["rev-parse", "@{upstream}"])?;
    if head == upstream_head {
        return Ok(format!(
            "primary sync: {branch} is current with {upstream} ({})",
            short_oid(&head)
        ));
    }
    if git_is_ancestor(root, &upstream_head, &head)? {
        return Ok(format!(
            "primary sync: local {branch} is ahead of {upstream}; using {}",
            short_oid(&head)
        ));
    }
    if !git_is_ancestor(root, &head, &upstream_head)? {
        return Err(PrimarySyncError::new(
            "device.sync_diverged",
            format!("{branch} and {upstream} have diverged; Relay will not merge or overwrite either side"),
        ));
    }

    let removed = git_capture(root, &["diff", "--diff-filter=DR", "--name-only", "HEAD..@{upstream}"])?;
    let safety_ref = if removed.is_empty() {
        None
    } else {
        let reference = format!("refs/relay/snapshots/device-sync-{}", short_oid(&head));
        worktree::git_mutate(root, &["update-ref", &reference, &head]).map_err(|error| {
            PrimarySyncError::new(
                "device.sync_snapshot_failed",
                format!("could not preserve {} before updating: {error}", short_oid(&head)),
            )
        })?;
        Some((reference, removed.lines().count()))
    };

    worktree::git_mutate(root, &["merge", "--ff-only", "--quiet", "@{upstream}"]).map_err(|error| {
        PrimarySyncError::new(
            "device.sync_fast_forward_failed",
            format!("could not safely fast-forward {branch}: {error}"),
        )
    })?;
    let updated = git_capture(root, &["rev-parse", "HEAD"])?;
    Ok(if let Some((reference, count)) = safety_ref {
        format!(
            "primary sync: fast-forwarded {branch} {} -> {}; preserved {count} removed or renamed path(s) at {reference}",
            short_oid(&head),
            short_oid(&updated)
        )
    } else {
        format!(
            "primary sync: fast-forwarded {branch} {} -> {}",
            short_oid(&head),
            short_oid(&updated)
        )
    })
}

struct RunWorkerRequest {
    root: PathBuf,
    command: String,
    device: String,
    adb: String,
    sdk_root: Option<PathBuf>,
    launch_after_build: bool,
    variant: String,
    _gradle_init: Option<tempfile::NamedTempFile>,
    sync_primary: bool,
}

fn run_worker(engine: Arc<Engine>, runtime: Arc<RunRuntime>, parent: uuid::Uuid, request: RunWorkerRequest) {
    let RunWorkerRequest {
        root,
        command,
        device,
        adb,
        sdk_root,
        launch_after_build,
        variant,
        _gradle_init,
        sync_primary,
    } = request;
    if sync_primary {
        match sync_primary_checkout(&root) {
            Ok(message) => runtime.push(message),
            Err(error) => {
                fail_run(&engine, &runtime, parent, error.code, &error.message);
                return;
            }
        }
    }
    let env = [("RELAY_DEVICE", device.as_str()), ("ANDROID_SERIAL", device.as_str())];
    match stream_command(
        &engine,
        &runtime,
        parent,
        &root,
        &command,
        &env,
        sdk_root.as_deref(),
        "build or install command failed",
    ) {
        Streamed::Ok => {}
        Streamed::Stopped | Streamed::Failed => return,
    }
    let _ = Command::new(&adb)
        .args(["-s", &device, "logcat", "-c"])
        .output();
    let launched = if launch_after_build {
        match launch_installed_app(&adb, &device, &root, &variant) {
            Ok(app) => {
                runtime.push(format!("app launched: {}", app.component));
                Some(app)
            }
            Err(message) => {
                runtime.push(message);
                fail_run(
                    &engine,
                    &runtime,
                    parent,
                    "device.launch_failed",
                    "installed app could not be opened",
                );
                return;
            }
        }
    } else {
        variant_application_ids(&root, &variant)
            .into_iter()
            .next()
            .map(|package| LaunchedApp { component: package.clone(), package })
    };
    let Some(launched) = launched else {
        fail_run(
            &engine,
            &runtime,
            parent,
            "device.package_missing",
            "no applicationId was found for the installed variant",
        );
        return;
    };
    let Some(pid) = wait_for_app_pid(&adb, &device, &launched.package, Duration::from_secs(10)) else {
        fail_run(
            &engine,
            &runtime,
            parent,
            "device.pid_missing",
            "the installed app did not expose a process after launch",
        );
        return;
    };
    if advance_run(&engine, runtime.id, parent, "running", false).is_err() {
        return;
    }
    runtime.push(format!("logcat attached pid={pid}"));
    let mut logcat = Command::new(&adb);
    logcat
        .args([
            "-s",
            &device,
            "logcat",
            "-v",
            "brief",
            "-b",
            "main,system,crash",
            &format!("--pid={pid}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut child) = logcat.spawn() else {
        fail_run(
            &engine,
            &runtime,
            parent,
            "device.logcat_failed",
            "could not start logcat",
        );
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        fail_run(
            &engine,
            &runtime,
            parent,
            "device.logcat_failed",
            "logcat had no output stream",
        );
        return;
    };
    runtime.set_child(child);
    let mut crash_reported = false;
    for line in BufReader::new(stdout).lines() {
        if runtime.stopped() {
            return;
        }
        let Ok(line) = line else { break };
        if !crash_reported && line.contains("FATAL EXCEPTION") {
            crash_reported = true;
            report_crash(&engine, runtime.id, parent, &line);
        }
        runtime.push(line);
    }
    if let Some(mut child) = runtime.take_child() {
        let _ = child.wait();
    }
    if !runtime.stopped() {
        let _ = advance_run(&engine, runtime.id, parent, "finished", true);
    }
    engine.device_runs.lock().unwrap().remove(&runtime.id);
    super::device_lease::release_run(&engine, runtime.id);
}

/// Outcome of one streamed shell command. `Failed` has already failed the run; `Stopped`
/// means `device.run.stop` won the race and the worker owns nothing more.
enum Streamed {
    Ok,
    Stopped,
    Failed,
}

/// Run one shell command inside `root`, streaming its output into the run buffer. Shared by
/// device runs and release builds so both stop, fail and stream identically.
#[allow(clippy::too_many_arguments)]
fn stream_command(
    engine: &Arc<Engine>,
    runtime: &Arc<RunRuntime>,
    parent: uuid::Uuid,
    root: &Path,
    command: &str,
    env: &[(&str, &str)],
    sdk_root: Option<&Path>,
    failure: &str,
) -> Streamed {
    runtime.push(format!("$ {command}"));
    // Keep bus tests independent of the developer's interactive shell. Production
    // run commands still use Fish, while the test instance only executes portable
    // fixture commands and must also work on bare CI runners.
    let shell = if engine.instance == Instance::Test { "sh" } else { "fish" };
    let mut spawn = Command::new(shell);
    spawn
        .current_dir(root)
        .args(["-lc", &format!("{command} 2>&1")])
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(sdk_root) = sdk_root {
        spawn.env("ANDROID_HOME", sdk_root).env("ANDROID_SDK_ROOT", sdk_root);
    }
    let Ok(mut child) = spawn.spawn() else {
        fail_run(engine, runtime, parent, "device.run_spawn_failed", "could not start the run command");
        return Streamed::Failed;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        fail_run(engine, runtime, parent, "device.run_spawn_failed", "run command had no output stream");
        return Streamed::Failed;
    };
    runtime.set_child(child);
    for line in BufReader::new(stdout).lines() {
        if runtime.stopped() {
            return Streamed::Stopped;
        }
        match line {
            Ok(line) => runtime.push(line),
            Err(error) => {
                runtime.push(format!("output: {error}"));
                break;
            }
        }
    }
    let status = runtime.take_child().and_then(|mut child| child.wait().ok());
    if runtime.stopped() {
        return Streamed::Stopped;
    }
    if !status.is_some_and(|status| status.success()) {
        fail_run(engine, runtime, parent, "device.build_failed", failure);
        return Streamed::Failed;
    }
    Streamed::Ok
}

struct BuildWorkerRequest {
    root: PathBuf,
    command: String,
    sdk_root: PathBuf,
    variant: String,
    format: String,
    signing_profile: Option<SigningRuntimeProfile>,
    _signing_init: Option<tempfile::NamedTempFile>,
}

/// A release build is one Gradle task and the artifact it left behind. No device is
/// involved, so the run finishes where a device run would start logcat (D152).
fn build_worker(engine: Arc<Engine>, runtime: Arc<RunRuntime>, parent: uuid::Uuid, request: BuildWorkerRequest) {
    let BuildWorkerRequest { root, command, sdk_root, variant, format, signing_profile, _signing_init } = request;
    let signing_password = match signing_profile.as_ref() {
        Some(profile) => match load_signing_secret(engine.instance, profile) {
            Ok(password) => Some(password),
            Err(error) => {
                fail_run(&engine, &runtime, parent, &error.code, &error.message);
                return;
            }
        },
        None => None,
    };
    let keystore = signing_profile.as_ref().map(|profile| profile.keystore.display().to_string());
    let env = match (&signing_profile, &signing_password, &keystore) {
        (Some(profile), Some(password), Some(keystore)) => vec![
            ("RELAY_SIGNING_STORE_FILE", keystore.as_str()),
            ("RELAY_SIGNING_STORE_PASSWORD", password.as_str()),
            ("RELAY_SIGNING_KEY_ALIAS", profile.key_alias.as_str()),
            ("RELAY_SIGNING_KEY_PASSWORD", password.as_str()),
        ],
        _ => Vec::new(),
    };
    if let Some(profile) = &signing_profile {
        runtime.push(format!("Relay signing: {} ({})", profile.key_alias, profile.keystore.display()));
    }
    match stream_command(
        &engine,
        &runtime,
        parent,
        &root,
        &command,
        &env,
        Some(&sdk_root),
        "the Gradle build failed",
    ) {
        Streamed::Ok => {}
        Streamed::Stopped | Streamed::Failed => return,
    }
    let Some(artifact) = variant_artifact(&root, &variant, &format) else {
        fail_run(
            &engine,
            &runtime,
            parent,
            "device.artifact_missing",
            &format!("the build wrote no {format} under build/outputs for variant {variant}"),
        );
        return;
    };
    let signing = artifact_signing(&sdk_root, &artifact, &format);
    runtime.push(format!("signing: {signing}"));
    runtime.push(format!("artifact: {}", artifact.display()));
    let _ = finish_build(&engine, runtime.id, parent, &artifact, signing);
    engine.device_runs.lock().unwrap().remove(&runtime.id);
}

/// Verify the artifact after Gradle writes it. Relay never asks for a keystore or password:
/// the target project's release signing config owns those, and this only checks the result.
fn artifact_signing(sdk_root: &Path, artifact: &Path, format: &str) -> &'static str {
    let output = if format == "apk" {
        let Some(apksigner) = latest_build_tool(sdk_root, "apksigner") else { return "unverified" };
        Command::new(apksigner).args(["verify", artifact.to_string_lossy().as_ref()]).env("LC_ALL", "C").output()
    } else {
        let jarsigner = std::env::var_os("JAVA_HOME")
            .map(PathBuf::from)
            .map(|home| home.join("bin/jarsigner"))
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("jarsigner"));
        Command::new(jarsigner).args(["-verify", artifact.to_string_lossy().as_ref()]).env("LC_ALL", "C").output()
    };
    let Ok(output) = output else { return "unverified" };
    let detail = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).to_ascii_lowercase();
    if (format == "apk" && output.status.success()) || (format == "bundle" && detail.contains("jar verified")) {
        "signed"
    } else if detail.contains("no jar signatures") || detail.contains("not signed") || detail.contains("jar is unsigned") {
        "unsigned"
    } else {
        "unverified"
    }
}

fn latest_build_tool(sdk_root: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(sdk_root.join("build-tools"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(name))
        .filter(|path| path.is_file())
        .max()
}

/// The artifact a finished build left behind: the newest `.apk` / `.aab` under any module's
/// `build/outputs/{apk,bundle}`. Flavoured variants nest one directory deeper than the
/// variant name, so a missing variant directory falls back to the whole outputs tree.
fn variant_artifact(root: &Path, variant: &str, format: &str) -> Option<PathBuf> {
    let (outputs, extension) = if format == "bundle" {
        ("outputs/bundle", "aab")
    } else {
        ("outputs/apk", "apk")
    };
    let mut found = Vec::new();
    collect_build_outputs(root, outputs, variant, extension, 0, &mut found);
    found
        .into_iter()
        .max_by_key(|path| std::fs::metadata(path).and_then(|data| data.modified()).ok())
}

fn collect_build_outputs(dir: &Path, outputs: &str, variant: &str, extension: &str, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "build" {
            let outputs = path.join(outputs);
            let scoped = outputs.join(variant);
            collect_files(if scoped.is_dir() { &scoped } else { &outputs }, extension, 0, found);
        } else if !matches!(name.as_ref(), ".git" | ".relay" | "node_modules" | "target") {
            collect_build_outputs(&path, outputs, variant, extension, depth + 1, found);
        }
    }
}

fn collect_files(dir: &Path, extension: &str, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            collect_files(&path, extension, depth + 1, found);
        } else if path.extension().is_some_and(|found| found == extension) {
            found.push(path);
        }
    }
}

struct LaunchedApp {
    component: String,
    package: String,
}

fn launch_installed_app(adb: &str, device: &str, root: &Path, variant: &str) -> Result<LaunchedApp, String> {
    let packages = variant_application_ids(root, variant);
    if packages.is_empty() {
        return Err(format!(
            "no applicationId found in Gradle output metadata for variant {variant}"
        ));
    }
    let mut resolution_errors = Vec::new();
    for package in packages {
        let resolved = Command::new(adb)
            .args([
                "-s",
                device,
                "shell",
                "cmd",
                "package",
                "resolve-activity",
                "--brief",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.LAUNCHER",
                &package,
            ])
            .output()
            .map_err(|error| format!("could not resolve launcher for {package}: {error}"))?;
        let stdout = String::from_utf8_lossy(&resolved.stdout);
        let component = stdout
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with(&format!("{package}/")))
            .map(str::to_string);
        let Some(component) = component.filter(|_| resolved.status.success()) else {
            let detail = String::from_utf8_lossy(&resolved.stderr).trim().to_string();
            resolution_errors.push(if detail.is_empty() { package } else { format!("{package}: {detail}") });
            continue;
        };
        let launched = Command::new(adb)
            .args(["-s", device, "shell", "am", "start", "-W", "-n", &component])
            .output()
            .map_err(|error| format!("could not launch {component}: {error}"))?;
        let stdout = String::from_utf8_lossy(&launched.stdout);
        let stderr = String::from_utf8_lossy(&launched.stderr);
        let refused = !launched.status.success()
            || stdout.contains("Error:")
            || stderr.contains("Error:")
            || stdout.contains("unable to resolve Intent")
            || stderr.contains("unable to resolve Intent");
        if refused {
            let detail = format!("{}\n{}", stdout.trim(), stderr.trim()).trim().to_string();
            return Err(format!("launch refused for {component}: {detail}"));
        }
        return Ok(LaunchedApp { component, package });
    }
    Err(format!(
        "no launcher activity resolved for installed package(s): {}",
        resolution_errors.join(", ")
    ))
}

fn wait_for_app_pid(
    adb: &str,
    device: &str,
    package: &str,
    timeout: Duration,
) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    loop {
        let output = Command::new(adb)
            .args(["-s", device, "shell", "pidof", "-s", package])
            .output()
            .ok();
        if let Some(pid) = output
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|text| text.split_whitespace().find_map(|value| value.parse::<u32>().ok()))
        {
            return Some(pid);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn variant_application_ids(root: &Path, variant: &str) -> Vec<String> {
    let mut metadata = Vec::new();
    collect_apk_metadata(root, 0, &mut metadata);
    let mut packages = metadata
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|value| value.get("variantName").and_then(Value::as_str).is_some_and(|name| name.eq_ignore_ascii_case(variant)))
        .filter_map(|value| value.get("applicationId").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    packages.sort();
    packages.dedup();
    packages
}

fn collect_apk_metadata(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 5 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_dir() { continue; }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "build" {
            collect_output_metadata(&path.join("outputs/apk"), 0, found);
        } else if !matches!(name.as_ref(), ".git" | ".relay" | "node_modules" | "target") {
            collect_apk_metadata(&path, depth + 1, found);
        }
    }
}

fn collect_output_metadata(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 4 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "output-metadata.json") {
            found.push(path);
        } else if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            collect_output_metadata(&path, depth + 1, found);
        }
    }
}

fn fail_run(engine: &Engine, runtime: &RunRuntime, parent: uuid::Uuid, code: &str, message: &str) {
    runtime.push(format!("{code}: {message}"));
    let _ = advance_run(engine, runtime.id, parent, "failed", true);
    engine.device_runs.lock().unwrap().remove(&runtime.id);
    super::device_lease::release_run(engine, runtime.id);
}

/// A finished build records its artifact in the same write that finishes the run, so
/// `device.run.list` never shows a finished build without the file it produced.
fn finish_build(engine: &Engine, id: Id, parent: uuid::Uuid, artifact: &Path, signing: &str) -> Result<(), BusError> {
    let project_id = {
        let conn = engine.store.lock();
        conn.query_row("SELECT project_id FROM device_runs WHERE id=?1", [id], |row| row.get::<_, Id>(0)).bus()?
    };
    let artifact = artifact.display().to_string();
    engine.system_write("device.build.finish",Some(parent),Some(project_id),None,json!({"run_id":id,"artifact":artifact,"signing":signing}),|tx,now|{
        tx.execute("UPDATE device_runs SET state='finished',finished_at=?1,artifact=?2,signing=?3 WHERE id=?4",params![now,artifact,signing,id]).bus()?;
        let run=get_run(tx,id)?;
        Ok(((),vec![("run.changed".into(),serde_json::to_value(run).bus()?)]))
    })
}

fn advance_run(
    engine: &Engine,
    id: Id,
    parent: uuid::Uuid,
    state: &str,
    finished: bool,
) -> Result<(), BusError> {
    let project_id = {
        let conn = engine.store.lock();
        conn.query_row(
            "SELECT project_id FROM device_runs WHERE id=?1",
            [id],
            |row| row.get::<_, Id>(0),
        )
        .bus()?
    };
    engine.system_write("device.run.advance",Some(parent),Some(project_id),None,json!({"run_id":id,"state":state}),|tx,now|{
        tx.execute("UPDATE device_runs SET state=?1,finished_at=CASE WHEN ?2 THEN ?3 ELSE finished_at END WHERE id=?4",params![state,finished as i64,now,id]).bus()?;
        let run=get_run(tx,id)?;
        Ok(((),vec![("run.changed".into(),serde_json::to_value(run).bus()?)]))
    })
}

fn report_crash(engine: &Engine, id: Id, parent: uuid::Uuid, line: &str) {
    let project_id = {
        let conn = engine.store.lock();
        conn.query_row(
            "SELECT project_id FROM device_runs WHERE id=?1",
            [id],
            |row| row.get::<_, Id>(0),
        )
        .ok()
    };
    let Some(project_id) = project_id else { return };
    let _=engine.system_write("device.run.crash",Some(parent),Some(project_id),None,json!({"run_id":id}),|tx,now|{
        tx.execute("INSERT INTO notifications(project_id,category,title,body,link,read,created_at) VALUES (?1,'system','App crashed on device',?2,NULL,0,?3)",params![project_id,line,now]).bus()?;
        Ok(((),vec![("run.crash".into(),json!({"run_id":id,"line":line})),("notify.new".into(),json!({"category":"system","project_id":project_id,"run_id":id}))]))
    });
}

fn get_run(conn: &rusqlite::Connection, id: Id) -> Result<Run, BusError> {
    conn.query_row("SELECT * FROM device_runs WHERE id=?1", [id], run_row)
        .optional()
        .bus()?
        .ok_or_else(|| BusError::not_found("device.run_not_found", format!("no run {id}")))
}
fn run_row(row: &Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        kind: row.get("kind")?,
        device: row.get("device")?,
        worktree: row.get("worktree")?,
        state: row.get("state")?,
        artifact: row.get("artifact")?,
        variant: row.get("variant")?,
        format: row.get("format")?,
        publish: row.get::<_, i64>("publish")? != 0,
        signing: row.get("signing")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn commit(root: &Path, message: &str) {
        git(root, &["add", "-A"]);
        git(
            root,
            &[
                "-c",
                "user.name=Relay Test",
                "-c",
                "user.email=relay@example.invalid",
                "commit",
                "-m",
                message,
            ],
        );
    }

    fn upstream_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let remote = fixture.path().join("remote.git");
        let seed = fixture.path().join("seed");
        let checkout = fixture.path().join("checkout");
        let init = Command::new("git")
            .args(["init", "--bare", "--initial-branch=main"])
            .arg(&remote)
            .output()
            .unwrap();
        assert!(init.status.success());
        let init = Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(&seed)
            .output()
            .unwrap();
        assert!(init.status.success());
        std::fs::write(seed.join("keep.txt"), "base\n").unwrap();
        commit(&seed, "base");
        git(&seed, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&seed, &["push", "-u", "origin", "main"]);
        let cloned = Command::new("git")
            .args(["clone", remote.to_str().unwrap(), checkout.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(cloned.status.success());
        (fixture, seed, checkout)
    }

    fn push_file(seed: &Path, path: &str, contents: &str, message: &str) {
        std::fs::write(seed.join(path), contents).unwrap();
        commit(seed, message);
        git(seed, &["push"]);
    }

    #[test]
    fn sizes_preserve_aspect_and_round_like_the_server() {
        // scrcpy rounds the short side to a multiple of 8, not 2 (486 was never what it sent).
        assert_eq!(mirror::fit_size(1080, 2400, 1080), (488, 1080));
        assert_eq!(mirror::fit_size(800, 600, 1080), (800, 600));
    }

    #[test]
    fn primary_sync_fast_forwards_a_clean_checkout() {
        let (_fixture, seed, checkout) = upstream_fixture();
        push_file(&seed, "new.txt", "remote\n", "remote update");

        let message = sync_primary_checkout(&checkout).unwrap();

        assert!(message.contains("fast-forwarded main"), "{message}");
        assert_eq!(std::fs::read_to_string(checkout.join("new.txt")).unwrap(), "remote\n");
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), git(&seed, &["rev-parse", "HEAD"]));
    }

    #[test]
    fn primary_sync_refuses_dirty_or_diverged_checkouts() {
        let (_fixture, seed, checkout) = upstream_fixture();
        std::fs::write(checkout.join("keep.txt"), "local dirty\n").unwrap();
        let dirty = sync_primary_checkout(&checkout).unwrap_err();
        assert_eq!(dirty.code, "device.sync_dirty");
        assert_eq!(std::fs::read_to_string(checkout.join("keep.txt")).unwrap(), "local dirty\n");

        git(&checkout, &["checkout", "--", "keep.txt"]);
        std::fs::write(checkout.join("local.txt"), "local\n").unwrap();
        commit(&checkout, "local update");
        push_file(&seed, "remote.txt", "remote\n", "remote update");
        let diverged = sync_primary_checkout(&checkout).unwrap_err();
        assert_eq!(diverged.code, "device.sync_diverged");
        assert!(checkout.join("local.txt").is_file());
        assert!(!checkout.join("remote.txt").exists());
    }

    #[test]
    fn primary_sync_snapshots_before_remote_deletions() {
        let (_fixture, seed, checkout) = upstream_fixture();
        let old_head = git(&checkout, &["rev-parse", "HEAD"]);
        git(&seed, &["rm", "keep.txt"]);
        commit(&seed, "remove tracked file");
        git(&seed, &["push"]);

        let message = sync_primary_checkout(&checkout).unwrap();

        assert!(message.contains("preserved 1 removed or renamed path"), "{message}");
        assert!(!checkout.join("keep.txt").exists());
        let reference = format!("refs/relay/snapshots/device-sync-{}", short_oid(&old_head));
        assert_eq!(git(&checkout, &["rev-parse", &reference]), old_head);
        assert_eq!(git(&checkout, &["show", &format!("{reference}:keep.txt")]), "base");
    }

    #[test]
    fn primary_sync_reports_fetch_failures_without_changing_head() {
        let (_fixture, _seed, checkout) = upstream_fixture();
        let head = git(&checkout, &["rev-parse", "HEAD"]);
        git(&checkout, &["remote", "set-url", "origin", "/definitely/missing/relay.git"]);

        let error = sync_primary_checkout(&checkout).unwrap_err();

        assert_eq!(error.code, "device.sync_fetch_failed");
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), head);
        assert!(checkout.join("keep.txt").is_file());
    }
}
