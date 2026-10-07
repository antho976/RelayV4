//! `remote.json` under concurrent writers: the door admitting phones on many connections while
//! `relay remote pair` and `revoke` run in other processes.

use relay_remote::registry::Presented;
use relay_remote::Registry;
use std::time::Duration;

#[test]
fn concurrent_updates_never_lose_an_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.json");
    Registry::update(&path, |_| Ok::<_, anyhow::Error>(())).unwrap();

    // Each thread pairs devices one read-modify-write at a time, as admissions do.
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                let mut ids = Vec::new();
                for i in 0..25 {
                    let id = Registry::update(&path, |r| {
                        let code = r.begin_pair(false).code;
                        match r.present(&code, &format!("phone {t}-{i}"), "test") {
                            Presented::Paired(device) => Ok(device.id),
                            other => Err(anyhow::anyhow!("{other:?}")),
                        }
                    })
                    .unwrap();
                    ids.push(id);
                }
                ids
            })
        })
        .collect();
    let ids: Vec<String> = threads.into_iter().flat_map(|t| t.join().unwrap()).collect();
    let reg = Registry::load(&path).unwrap();
    assert_eq!(reg.devices.len(), 200);
    for id in &ids {
        assert!(reg.device(id).is_some(), "device {id} was lost");
    }

    // Nothing is left behind: no temp file of any write survives it.
    let stray: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "remote.json" && n != "remote.json.lock")
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
}

#[test]
fn an_update_waits_for_another_process_holding_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.json");
    let device = Registry::update(&path, |r| {
        let code = r.begin_pair(false).code;
        match r.present(&code, "Pixel", "test") {
            Presented::Paired(d) => Ok(d.id),
            other => Err(anyhow::anyhow!("{other:?}")),
        }
    })
    .unwrap();

    // Another process — a separate descriptor on the same lock file — is mid-update.
    let held = std::fs::OpenOptions::new().write(true).open(Registry::lock_path(&path)).unwrap();
    held.lock().unwrap();
    let revoker = {
        let (path, device) = (path.clone(), device.clone());
        std::thread::spawn(move || Registry::update(&path, |r| Ok::<_, anyhow::Error>(r.revoke(&device))).unwrap())
    };
    std::thread::sleep(Duration::from_millis(200));
    assert!(!revoker.is_finished(), "the update did not wait for the lock");
    assert!(Registry::load(&path).unwrap().device(&device).is_some());
    // That process saves what it loaded — the device still paired — and lets go.
    Registry::load(&path).unwrap().save(&path).unwrap();
    drop(held);
    assert!(revoker.join().unwrap(), "the revoke ran against the latest file");
    assert!(Registry::load(&path).unwrap().device(&device).is_none());
}

#[test]
fn a_failed_change_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote.json");
    let err = Registry::update(&path, |r| {
        r.host_name = "changed".into();
        Err::<(), _>(anyhow::anyhow!("refused"))
    });
    assert!(err.is_err());
    assert!(!path.exists(), "a refused change wrote the file");
}
