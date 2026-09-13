//! Event-driven worktree watchers. A short trailing debounce folds editor saves and git's
//! lock/rename sequence into one refresh event without introducing an idle polling loop.

use crate::engine::{Ctx, Engine, Unlocked};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// A status refresh can rewrite only index stat fields. Compare the staged tree
// rather than index bytes so this cache maintenance does not trigger another scan.
fn index_signature(root: &Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let repo = gix::open(root).ok()?;
    let index = repo.index().ok()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for entry in index.entries() {
        entry.path(&index).hash(&mut hash);
        entry.id.hash(&mut hash);
        entry.mode.bits().hash(&mut hash);
        entry.flags.bits().hash(&mut hash);
    }
    Some(hash.finish())
}

/// Both request contexts can defer work: [`Ctx`] until after its transaction commits, [`Unlocked`]
/// until its handler returns. Watcher registration only cares that the store lock is not held.
pub(crate) trait Defer {
    fn defer(&mut self, f: Box<dyn FnOnce(Arc<Engine>) + Send + 'static>);
}
impl Defer for Ctx<'_> {
    fn defer(&mut self, f: Box<dyn FnOnce(Arc<Engine>) + Send + 'static>) {
        self.after_commit(f);
    }
}
impl Defer for Unlocked<'_> {
    fn defer(&mut self, f: Box<dyn FnOnce(Arc<Engine>) + Send + 'static>) {
        self.after_commit(f);
    }
}

/// Recursive inotify registration may enumerate a large worktree. Start it after the current bus
/// transaction releases the store lock so first-time registration cannot block PTY or UI requests.
pub(crate) fn ensure_after_commit(
    ctx: &mut impl Defer,
    root: PathBuf,
    project_id: relay_bus::types::Id,
) {
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    ctx.defer(Box::new(move |engine| {
        let key = root.display().to_string();
        if engine.watchers.lock().unwrap().contains_key(&key) {
            return;
        }
        if !engine
            .watcher_registrations
            .lock()
            .unwrap()
            .insert(key.clone())
        {
            return;
        }
        let thread_engine = engine.clone();
        if std::thread::Builder::new()
            .name("watch-register".into())
            .spawn(move || ensure(&thread_engine, &root, project_id))
            .is_err()
        {
            engine.watcher_registrations.lock().unwrap().remove(&key);
        }
    }));
}

pub fn ensure(engine: &Engine, root: &Path, project_id: relay_bus::types::Id) {
    let key = std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string();
    let Some(engine) = engine.arc() else { return };
    let weak = Arc::downgrade(&engine);
    let pending = Arc::new(AtomicBool::new(false));
    let pending_cb = pending.clone();
    let root = PathBuf::from(&key);
    let callback_root = root.clone();
    let mut last_index = index_signature(&root);
    let watcher: notify::Result<RecommendedWatcher> =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            // Refreshing the tree and Git state reads these same watched files.
            // Access notifications must not turn one mutation into an idle refresh loop.
            if matches!(event.kind, notify::EventKind::Access(_)) && !event.need_rescan() {
                return;
            }
            if (!event.need_rescan()
                && event
                    .paths
                    .iter()
                    .all(|path| {
                        if path == &callback_root.join(".git/index") {
                            let next = index_signature(&callback_root);
                            let unchanged = next.is_some() && next == last_index;
                            last_index = next;
                            unchanged
                        } else {
                            is_generated_path(&callback_root, path)
                        }
                    }))
                || pending_cb.swap(true, Ordering::SeqCst)
            {
                return;
            }
            let weak = weak.clone();
            let pending = pending_cb.clone();
            let root = callback_root.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(125));
                pending.store(false, Ordering::SeqCst);
                if let Some(engine) = weak.upgrade() {
                    let payload =
                        json!({"project_id": project_id, "worktree": root.display().to_string()});
                    // A filesystem mutation makes both the tree and Git projection stale. One
                    // scoped event lets the Code workspace refresh both without a guaranteed
                    // duplicate pass. Explicit git.* mutations still emit git.changed.
                    engine.emit_system("file.changed", payload);
                }
            });
        });
    let registered = if let Ok(mut watcher) = watcher {
        if watcher.watch(&root, RecursiveMode::Recursive).is_ok() {
            let mut watchers = engine.watchers.lock().unwrap();
            if watchers.contains_key(&key) {
                false
            } else {
                watchers.insert(key.clone(), watcher);
                true
            }
        } else {
            false
        }
    } else {
        false
    };
    engine.watcher_registrations.lock().unwrap().remove(&key);
    if registered {
        engine.emit_system(
            "file.changed",
            json!({"project_id": project_id, "worktree": root.display().to_string()}),
        );
    }
}

/// Build caches can change hundreds of times per second while Relay itself is compiling. They are
/// never useful refresh signals for the source tree and would otherwise keep the single bus busy.
pub(crate) fn is_generated_path(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    // Status filters may write .git/lfs and objects; these writes are not source
    // edits and must not start another status scan. Keep real index/ref changes.
    if let Ok(git_path) = relative.strip_prefix(".git") {
        return !(git_path == Path::new("index")
            || git_path == Path::new("HEAD")
            || git_path == Path::new("packed-refs")
            || git_path == Path::new("config")
            || (git_path.starts_with("refs") && !git_path.to_string_lossy().ends_with(".lock")));
    }
    relative.components().any(|component| {
        let value = component.as_os_str().to_string_lossy();
        matches!(
            value.as_ref(),
            ".relay"
                | "node_modules"
                | "target"
                | "build"
                | ".gradle"
                | ".svelte-kit"
                | ".next"
                | "dist"
                | "coverage"
                | "Intermediate"
                | "Saved"
                | "DerivedDataCache"
                | "Binaries"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::is_generated_path;
    use std::path::Path;

    #[tokio::test]
    async fn reading_the_tree_does_not_refresh_it_but_writing_does() {
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("source.txt");
        std::fs::write(&file, "before").unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@relay.test",
                "commit",
                "-qm",
                "Fixture",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        let engine =
            crate::Engine::new(crate::Instance::Test, crate::Store::open_memory().unwrap());
        let mut events = engine.subscribe();
        super::ensure(&engine, directory.path(), 1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap()
                .ev,
            "file.changed"
        );
        for _ in 0..10 {
            std::fs::read(&file).unwrap();
            std::fs::read_dir(directory.path())
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(std::process::Command::new("git")
                .env("GIT_OPTIONAL_LOCKS", "0")
                .arg("-C")
                .arg(directory.path())
                .args(["status", "--porcelain"])
                .output()
                .unwrap()
                .status
                .success());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(400), events.recv())
                .await
                .is_err(),
            "Read-only refresh caused another refresh"
        );
        std::fs::write(&file, "after").unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap()
                .ev,
            "file.changed"
        );
    }

    #[test]
    fn generated_build_paths_do_not_refresh_the_code_workspace() {
        let root = Path::new("/repo");
        assert!(is_generated_path(root, Path::new("/repo/target/debug/app")));
        assert!(is_generated_path(
            root,
            Path::new("/repo/apps/web/node_modules/pkg/index.js")
        ));
        assert!(is_generated_path(
            root,
            Path::new("/repo/app/build/generated/source.kt")
        ));
        assert!(!is_generated_path(
            root,
            Path::new("/repo/apps/web/src/App.svelte")
        ));
        assert!(!is_generated_path(root, Path::new("/repo/.git/HEAD")));
        for path in [".git/lfs/tmp/asset", ".git/objects/ab/object", ".git/index.lock",
            "Saved/Logs/Unreal.log", "Intermediate/Build/file", "DerivedDataCache/cache", "Binaries/Linux/game"] {
            assert!(is_generated_path(root, &root.join(path)), "{path}");
        }
        for path in [".git/index", ".git/refs/heads/main", "Content/Level.umap"] {
            assert!(!is_generated_path(root, &root.join(path)), "{path}");
        }
    }
}
