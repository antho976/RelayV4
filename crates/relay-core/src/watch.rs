//! Event-driven worktree watchers. A short trailing debounce folds editor saves and git's
//! lock/rename sequence into one refresh event without introducing an idle polling loop.
//!
//! Each directory is watched on its own (non-recursive inotify watches placed by our own walk),
//! so build output, engine caches and `.git/objects` never take a watch: inotify's recursive
//! mode has no filter, and an Unreal tree's `Intermediate/` and `DerivedDataCache/` alone could
//! use up `max_user_watches` for every other program on the machine. Directories created later
//! are added as their creation is seen.

use crate::engine::{Ctx, Engine, Unlocked};
use notify::event::{CreateKind, ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Worktrees watched at once. Past this the least recently used one is dropped; its tree still
/// works, with badges refreshed on a short timer instead of by events.
const MAX_ROOTS: usize = 16;
/// Directories watched per worktree. A tree past this is only partly watched, and says so once.
const MAX_DIRS: usize = 20_000;
/// A worktree whose watch failed (usually `max_user_watches`) is not retried before this.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
/// Changed paths reported in one `file.changed`. Past this `paths` is left out, which means
/// "anything in this worktree may have changed".
const MAX_PATHS: usize = 256;
const DEBOUNCE: Duration = Duration::from_millis(125);

struct Root {
    watcher: Arc<Mutex<RecommendedWatcher>>,
    dirs: Arc<AtomicUsize>,
    used: Instant,
}

/// The engine's worktree watchers, by canonical root.
#[derive(Default)]
pub(crate) struct Watchers {
    roots: HashMap<String, Root>,
    /// Roots whose registration failed, and when; see [`RETRY_AFTER`].
    failed: HashMap<String, Instant>,
}

/// Whether `root` (canonical, as the file handlers resolve it) has a live watcher.
pub(crate) fn is_watched(engine: &Engine, root: &Path) -> bool {
    engine.watchers.lock().unwrap().roots.contains_key(&root.display().to_string())
}

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

/// Watch registration walks the worktree. Start it after the current bus transaction releases
/// the store lock so first-time registration cannot block PTY or UI requests.
pub(crate) fn ensure_after_commit(
    ctx: &mut impl Defer,
    root: PathBuf,
    project_id: relay_bus::types::Id,
) {
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    ctx.defer(Box::new(move |engine| {
        let key = root.display().to_string();
        {
            let mut watchers = engine.watchers.lock().unwrap();
            if let Some(watched) = watchers.roots.get_mut(&key) {
                watched.used = Instant::now();
                return;
            }
            // A failed watch is not retried on every tree request: each retry walked the whole
            // tree again, and the limit it hit has not gone away.
            if watchers.failed.get(&key).is_some_and(|at| at.elapsed() < RETRY_AFTER) {
                return;
            }
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

/// What one debounce window collected.
#[derive(Default)]
struct Batch {
    scheduled: bool,
    paths: BTreeSet<String>,
    /// More than [`MAX_PATHS`] changed, or the kernel queue overflowed: report no `paths`.
    overflow: bool,
    new_dirs: Vec<PathBuf>,
}

pub fn ensure(engine: &Engine, root: &Path, project_id: relay_bus::types::Id) {
    let key = std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string();
    let Some(engine) = engine.arc() else { return };
    let weak = Arc::downgrade(&engine);
    let batch = Arc::new(Mutex::new(Batch::default()));
    let dirs = Arc::new(AtomicUsize::new(0));
    let root = PathBuf::from(&key);
    let callback_root = root.clone();
    let callback_key = key.clone();
    let mut last_index = index_signature(&root);
    // A build touches thousands of files, and every one of them arrives here as a path to
    // compare. Build the path being compared against once, not once per event path.
    let index_path = root.join(".git/index");
    let watcher: notify::Result<RecommendedWatcher> =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            // Refreshing the tree and Git state reads these same watched files.
            // Access notifications must not turn one mutation into an idle refresh loop.
            let rescan = event.need_rescan();
            if matches!(event.kind, EventKind::Access(_)) && !rescan {
                return;
            }
            let relevant: Vec<&PathBuf> = event
                .paths
                .iter()
                .filter(|path| {
                    if **path == index_path {
                        let next = index_signature(&callback_root);
                        let unchanged = next.is_some() && next == last_index;
                        last_index = next;
                        !unchanged
                    } else {
                        !is_generated_path(&callback_root, path)
                    }
                })
                .collect();
            if !rescan && relevant.is_empty() {
                return;
            }
            // Right away, not after the debounce: a tree request in the next 125 ms must not
            // be answered from badges computed before this change.
            crate::handlers::git::invalidate_badges(&callback_root);
            let may_add_dir = matches!(
                event.kind,
                EventKind::Create(CreateKind::Folder | CreateKind::Any)
                    | EventKind::Modify(ModifyKind::Name(RenameMode::To | RenameMode::Both))
            );
            let mut pending = batch.lock().unwrap();
            pending.overflow |= rescan;
            for path in relevant {
                if !pending.overflow {
                    let relative = path.strip_prefix(&callback_root).unwrap_or(path);
                    pending.paths.insert(relative.to_string_lossy().replace('\\', "/"));
                    if pending.paths.len() > MAX_PATHS {
                        pending.overflow = true;
                        pending.paths.clear();
                    }
                }
                if may_add_dir && should_watch(&callback_root, path) && path.is_dir() {
                    pending.new_dirs.push(path.clone());
                }
            }
            if std::mem::replace(&mut pending.scheduled, true) {
                return;
            }
            drop(pending);
            let weak = weak.clone();
            let batch = batch.clone();
            let root = callback_root.clone();
            let key = callback_key.clone();
            std::thread::spawn(move || {
                std::thread::sleep(DEBOUNCE);
                let taken = std::mem::take(&mut *batch.lock().unwrap());
                if let Some(engine) = weak.upgrade() {
                    if !taken.new_dirs.is_empty() {
                        watch_new_dirs(&engine, &key, &root, taken.new_dirs);
                    }
                    let mut payload =
                        json!({"project_id": project_id, "worktree": root.display().to_string()});
                    if !taken.overflow {
                        payload["paths"] = json!(taken.paths);
                    }
                    // A filesystem mutation makes both the tree and Git projection stale. One
                    // scoped event lets the Code workspace refresh both without a guaranteed
                    // duplicate pass. Explicit git.* mutations still emit git.changed.
                    engine.emit_system_project("file.changed", project_id, payload);
                }
            });
        });
    let registered = match watcher.and_then(|mut watcher| {
        add_tree(&mut watcher, &root, &root, &dirs)?;
        Ok(watcher)
    }) {
        Ok(watcher) => {
            let mut evicted = Vec::new();
            let mut watchers = engine.watchers.lock().unwrap();
            watchers.failed.remove(&key);
            let fresh = !watchers.roots.contains_key(&key);
            if fresh {
                // Removed worktrees first, then the least recently used, so the count stays bounded.
                let gone: Vec<String> =
                    watchers.roots.keys().filter(|k| !Path::new(k).exists()).cloned().collect();
                evicted.extend(gone.iter().filter_map(|k| watchers.roots.remove(k)));
                while watchers.roots.len() >= MAX_ROOTS {
                    let Some(oldest) =
                        watchers.roots.iter().min_by_key(|(_, r)| r.used).map(|(k, _)| k.clone())
                    else {
                        break;
                    };
                    evicted.extend(watchers.roots.remove(&oldest));
                }
                watchers.roots.insert(
                    key.clone(),
                    Root { watcher: Arc::new(Mutex::new(watcher)), dirs, used: Instant::now() },
                );
            }
            drop(watchers);
            drop(evicted);
            fresh
        }
        Err(error) => {
            tracing::warn!(
                root = %key, error = %error,
                "cannot watch worktree; file changes refresh on demand, retrying in 5 minutes"
            );
            engine.watchers.lock().unwrap().failed.insert(key.clone(), Instant::now());
            false
        }
    };
    engine.watcher_registrations.lock().unwrap().remove(&key);
    if registered {
        // No `paths`: anything may have changed between the caller's read and the watch.
        engine.emit_system_project(
            "file.changed",
            project_id,
            json!({"project_id": project_id, "worktree": root.display().to_string()}),
        );
    }
}

/// Watch directories created (or moved in) after registration, with everything already inside.
fn watch_new_dirs(engine: &Engine, key: &str, root: &Path, new_dirs: Vec<PathBuf>) {
    let Some((watcher, dirs)) = engine
        .watchers
        .lock()
        .unwrap()
        .roots
        .get(key)
        .map(|r| (r.watcher.clone(), r.dirs.clone()))
    else {
        return;
    };
    let mut watcher = watcher.lock().unwrap();
    for dir in new_dirs {
        if let Err(error) = add_tree(&mut watcher, root, &dir, &dirs) {
            tracing::warn!(root = %key, error = %error, "cannot watch a new directory; it refreshes on demand");
            return;
        }
    }
}

/// Place one non-recursive watch on `start` and on every directory below it that
/// [`should_watch`] allows. A subdirectory that vanished or cannot be read is skipped; running out
/// of watches is an error, and the caller decides what that costs.
fn add_tree(
    watcher: &mut RecommendedWatcher,
    root: &Path,
    start: &Path,
    dirs: &AtomicUsize,
) -> notify::Result<()> {
    let mut stack = vec![start.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dirs.load(Ordering::Relaxed) >= MAX_DIRS {
            return Ok(());
        }
        match watcher.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => {}
            Err(error) if dir != start && skippable(&error) => continue,
            Err(error) => return Err(error),
        }
        if dirs.fetch_add(1, Ordering::Relaxed) + 1 == MAX_DIRS {
            tracing::warn!(root = %root.display(), limit = MAX_DIRS, "worktree has too many directories; the rest are not watched");
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            // `DirEntry::file_type` does not follow links, so a linked directory is not entered.
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let path = entry.path();
                if should_watch(root, &path) {
                    stack.push(path);
                }
            }
        }
    }
    Ok(())
}

fn skippable(error: &notify::Error) -> bool {
    match &error.kind {
        notify::ErrorKind::PathNotFound => true,
        notify::ErrorKind::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
        ),
        _ => false,
    }
}

/// Whether a directory gets a watch. Of `.git`, only the directory itself (index, `HEAD`,
/// `packed-refs`, config) and `refs/`; never objects, logs or LFS. Nested repositories' `.git`
/// and everything [`is_generated_path`] filters out of events are not watched either.
fn should_watch(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if let Ok(git) = relative.strip_prefix(".git") {
        return git.as_os_str().is_empty() || git.starts_with("refs");
    }
    !relative.components().any(|c| c.as_os_str() == ".git") && !is_generated_path(root, path)
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
    fn watches_skip_git_internals_nested_repositories_and_build_output() {
        let root = Path::new("/repo");
        for path in ["", ".git", ".git/refs", ".git/refs/heads/feature", "Content/Maps", "Source/Game"] {
            assert!(super::should_watch(root, &root.join(path)), "{path}");
        }
        for path in [".git/objects", ".git/objects/ab", ".git/logs", ".git/lfs/tmp", "Plugins/Sub/.git",
            "Intermediate", "Plugins/Foo/Binaries", "DerivedDataCache", "Saved/Logs", "node_modules/pkg", "target"] {
            assert!(!super::should_watch(root, &root.join(path)), "{path}");
        }
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
