//! The PC's copy of Avex's history, `gym.db` beside the store: its own SQLite file, as Tally's
//! ledger and Arbiter's book are. An import replaces every row in one transaction, so a read sees
//! the old history or the new one, never half of each. The engine keeps the loaded [`History`] in
//! memory and reads from it; this file is what survives a restart.

use crate::model::{Cardio, Exercise, Goal, History, Meta, Session, Set};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

pub type Result<T> = rusqlite::Result<T>;

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE session (
    id INTEGER PRIMARY KEY,
    avex_id INTEGER NOT NULL,
    day_key TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    active_seconds INTEGER NOT NULL,
    volume_lb REAL NOT NULL,
    pr_count INTEGER NOT NULL,
    set_count INTEGER NOT NULL,
    session_type TEXT NOT NULL,
    intensity TEXT NOT NULL,
    untracked INTEGER NOT NULL,
    tags TEXT NOT NULL,
    journal TEXT NOT NULL,
    mood TEXT NOT NULL
);
CREATE TABLE exercise (
    id INTEGER PRIMARY KEY,
    session INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    exercise_id TEXT NOT NULL,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    difficulty TEXT NOT NULL,
    skipped INTEGER NOT NULL,
    note TEXT NOT NULL,
    was_pr INTEGER NOT NULL,
    hit_full_target INTEGER NOT NULL,
    superset TEXT NOT NULL
);
CREATE INDEX exercise_session ON exercise(session);
CREATE TABLE lift_set (
    id INTEGER PRIMARY KEY,
    exercise INTEGER NOT NULL REFERENCES exercise(id) ON DELETE CASCADE,
    weight_lb REAL,
    weight_text TEXT NOT NULL,
    reps INTEGER NOT NULL,
    rpe REAL,
    completed_at INTEGER,
    set_type TEXT NOT NULL,
    difficulty_tag TEXT NOT NULL,
    duration_seconds INTEGER,
    assisted INTEGER NOT NULL,
    amrap INTEGER NOT NULL,
    to_failure INTEGER NOT NULL
);
CREATE INDEX lift_set_exercise ON lift_set(exercise);
CREATE TABLE cardio (
    id INTEGER PRIMARY KEY,
    date INTEGER NOT NULL,
    kind TEXT NOT NULL,
    duration_min INTEGER NOT NULL,
    distance_km REAL,
    effort TEXT NOT NULL,
    rest_reason TEXT NOT NULL,
    note TEXT NOT NULL,
    interval_count INTEGER,
    hr_zone TEXT NOT NULL,
    incline_pct REAL,
    laps INTEGER,
    elevation_m REAL,
    conditions TEXT NOT NULL
);
CREATE TABLE goal (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    target_key TEXT NOT NULL,
    target_value REAL,
    priority INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    completed_at INTEGER,
    archived_at INTEGER,
    source TEXT NOT NULL,
    note TEXT NOT NULL
);
"#];
const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub struct Gym {
    conn: Connection,
}

impl Gym {
    pub fn open(path: &Path) -> Result<Gym> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Gym::init(conn)
    }

    pub fn open_in_memory() -> Result<Gym> {
        Gym::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Gym> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let have: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if have > SCHEMA_VERSION {
            return Err(rusqlite::Error::InvalidParameterName(format!("gym.db is version {have}; this build reads up to {SCHEMA_VERSION}")));
        }
        let tx = conn.transaction()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(have as usize) {
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        }
        tx.commit()?;
        Ok(Gym { conn })
    }

    /// Whether an export was ever imported.
    pub fn imported(&self) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM meta WHERE key = 'imported_at'", [], |_| Ok(())).optional()?.is_some())
    }

    /// Replaces the whole copy with `h`.
    pub fn replace(&mut self, h: &History) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM lift_set; DELETE FROM exercise; DELETE FROM session; DELETE FROM cardio; DELETE FROM goal; DELETE FROM meta;")?;
        {
            let m = &h.meta;
            let mut meta = tx.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
            for (k, v) in [
                ("export_version", m.export_version.to_string()),
                ("exported_at", m.exported_at.clone()),
                ("app_version", m.app_version.clone()),
                ("imported_at", m.imported_at.clone()),
                ("file", m.file.clone()),
                ("use_kg", m.use_kg.to_string()),
                ("user_name", m.user_name.clone()),
                ("user_goal", m.user_goal.clone()),
                ("days_per_week", m.days_per_week.to_string()),
                ("first_day_monday", m.first_day_monday.to_string()),
            ] {
                meta.execute(params![k, v])?;
            }
            let mut session = tx.prepare(
                "INSERT INTO session (avex_id, day_key, started_at, finished_at, active_seconds, volume_lb, pr_count, set_count, session_type, intensity, untracked, tags, journal, mood)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?;
            let mut exercise = tx.prepare(
                "INSERT INTO exercise (session, exercise_id, name, position, difficulty, skipped, note, was_pr, hit_full_target, superset)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            let mut set = tx.prepare(
                "INSERT INTO lift_set (exercise, weight_lb, weight_text, reps, rpe, completed_at, set_type, difficulty_tag, duration_seconds, assisted, amrap, to_failure)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            for s in &h.sessions {
                session.execute(params![
                    s.id, s.day_key, s.started_at, s.finished_at, s.active_seconds, s.volume_lb, s.pr_count, s.set_count, s.session_type, s.intensity,
                    s.untracked, s.tags, s.journal, s.mood
                ])?;
                let sid = tx.last_insert_rowid();
                for e in &s.exercises {
                    exercise.execute(params![sid, e.exercise_id, e.name, e.position, e.difficulty, e.skipped, e.note, e.was_pr, e.hit_full_target, e.superset])?;
                    let eid = tx.last_insert_rowid();
                    for x in &e.sets {
                        set.execute(params![
                            eid, x.weight_lb, x.weight_text, x.reps, x.rpe, x.completed_at, x.set_type, x.difficulty_tag, x.duration_seconds, x.assisted, x.amrap,
                            x.to_failure
                        ])?;
                    }
                }
            }
            let mut cardio = tx.prepare(
                "INSERT INTO cardio (date, kind, duration_min, distance_km, effort, rest_reason, note, interval_count, hr_zone, incline_pct, laps, elevation_m, conditions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )?;
            for c in &h.cardio {
                cardio.execute(params![
                    c.date, c.kind, c.duration_min, c.distance_km, c.effort, c.rest_reason, c.note, c.interval_count, c.hr_zone, c.incline_pct, c.laps,
                    c.elevation_m, c.conditions
                ])?;
            }
            let mut goal = tx.prepare(
                "INSERT INTO goal (kind, target_key, target_value, priority, created_at, completed_at, archived_at, source, note) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for g in &h.goals {
                goal.execute(params![g.kind, g.target_key, g.target_value, g.priority, g.created_at, g.completed_at, g.archived_at, g.source, g.note])?;
            }
        }
        tx.commit()
    }

    /// Forgets everything imported.
    pub fn clear(&mut self) -> Result<()> {
        self.replace(&History::default())?;
        self.conn.execute("DELETE FROM meta", [])?;
        Ok(())
    }

    /// The whole copy.
    pub fn load(&self) -> Result<History> {
        let meta: HashMap<String, String> =
            self.conn.prepare("SELECT key, value FROM meta")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_>>()?;
        let text = |k: &str| meta.get(k).cloned().unwrap_or_default();
        let int = |k: &str| meta.get(k).and_then(|v| v.parse().ok()).unwrap_or(0);
        let yes = |k: &str, default: bool| meta.get(k).map_or(default, |v| v == "true");
        let meta = Meta {
            export_version: int("export_version"),
            exported_at: text("exported_at"),
            app_version: text("app_version"),
            imported_at: text("imported_at"),
            file: text("file"),
            use_kg: yes("use_kg", false),
            user_name: text("user_name"),
            user_goal: text("user_goal"),
            days_per_week: int("days_per_week"),
            first_day_monday: yes("first_day_monday", true),
        };
        let mut sets: HashMap<i64, Vec<Set>> = HashMap::new();
        let mut q = self.conn.prepare(
            "SELECT exercise, weight_lb, weight_text, reps, rpe, completed_at, set_type, difficulty_tag, duration_seconds, assisted, amrap, to_failure FROM lift_set ORDER BY id",
        )?;
        for r in q.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Set {
                    weight_lb: r.get(1)?,
                    weight_text: r.get(2)?,
                    reps: r.get(3)?,
                    rpe: r.get(4)?,
                    completed_at: r.get(5)?,
                    set_type: r.get(6)?,
                    difficulty_tag: r.get(7)?,
                    duration_seconds: r.get(8)?,
                    assisted: r.get(9)?,
                    amrap: r.get(10)?,
                    to_failure: r.get(11)?,
                },
            ))
        })? {
            let (e, s) = r?;
            sets.entry(e).or_default().push(s);
        }
        let mut exercises: HashMap<i64, Vec<Exercise>> = HashMap::new();
        let mut q = self.conn.prepare(
            "SELECT id, session, exercise_id, name, position, difficulty, skipped, note, was_pr, hit_full_target, superset FROM exercise ORDER BY session, position, id",
        )?;
        for r in q.query_map([], |r| {
            Ok((
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(0)?,
                Exercise {
                    exercise_id: r.get(2)?,
                    name: r.get(3)?,
                    position: r.get(4)?,
                    difficulty: r.get(5)?,
                    skipped: r.get(6)?,
                    note: r.get(7)?,
                    was_pr: r.get(8)?,
                    hit_full_target: r.get(9)?,
                    superset: r.get(10)?,
                    sets: Vec::new(),
                },
            ))
        })? {
            let (session, id, mut e) = r?;
            e.sets = sets.remove(&id).unwrap_or_default();
            exercises.entry(session).or_default().push(e);
        }
        let mut q = self.conn.prepare(
            "SELECT id, avex_id, day_key, started_at, finished_at, active_seconds, volume_lb, pr_count, set_count, session_type, intensity, untracked, tags, journal, mood
             FROM session ORDER BY started_at, avex_id",
        )?;
        let sessions = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    Session {
                        id: r.get(1)?,
                        day_key: r.get(2)?,
                        started_at: r.get(3)?,
                        finished_at: r.get(4)?,
                        active_seconds: r.get(5)?,
                        volume_lb: r.get(6)?,
                        pr_count: r.get(7)?,
                        set_count: r.get(8)?,
                        session_type: r.get(9)?,
                        intensity: r.get(10)?,
                        untracked: r.get(11)?,
                        tags: r.get(12)?,
                        journal: r.get(13)?,
                        mood: r.get(14)?,
                        exercises: Vec::new(),
                    },
                ))
            })?
            .map(|r| {
                r.map(|(row, mut s)| {
                    s.exercises = exercises.remove(&row).unwrap_or_default();
                    s
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let cardio = self
            .conn
            .prepare(
                "SELECT date, kind, duration_min, distance_km, effort, rest_reason, note, interval_count, hr_zone, incline_pct, laps, elevation_m, conditions FROM cardio ORDER BY date, id",
            )?
            .query_map([], |r| {
                Ok(Cardio {
                    date: r.get(0)?,
                    kind: r.get(1)?,
                    duration_min: r.get(2)?,
                    distance_km: r.get(3)?,
                    effort: r.get(4)?,
                    rest_reason: r.get(5)?,
                    note: r.get(6)?,
                    interval_count: r.get(7)?,
                    hr_zone: r.get(8)?,
                    incline_pct: r.get(9)?,
                    laps: r.get(10)?,
                    elevation_m: r.get(11)?,
                    conditions: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        let goals = self
            .conn
            .prepare("SELECT kind, target_key, target_value, priority, created_at, completed_at, archived_at, source, note FROM goal ORDER BY id")?
            .query_map([], |r| {
                Ok(Goal {
                    kind: r.get(0)?,
                    target_key: r.get(1)?,
                    target_value: r.get(2)?,
                    priority: r.get(3)?,
                    created_at: r.get(4)?,
                    completed_at: r.get(5)?,
                    archived_at: r.get(6)?,
                    source: r.get(7)?,
                    note: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(History { meta, sessions, cardio, goals })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_and_load_round_trip() {
        let text = include_str!("../tests/avex_export.json");
        let h = crate::export::read(text, "avex_export.json", "2026-10-09T12:00:00-04:00").unwrap();
        let mut gym = Gym::open_in_memory().unwrap();
        assert!(!gym.imported().unwrap());
        gym.replace(&h).unwrap();
        assert!(gym.imported().unwrap());
        assert_eq!(gym.load().unwrap(), h);
        // A second import replaces, never adds.
        gym.replace(&h).unwrap();
        assert_eq!(gym.load().unwrap().sessions.len(), h.sessions.len());
        gym.clear().unwrap();
        assert!(!gym.imported().unwrap());
        assert!(gym.load().unwrap().sessions.is_empty());
    }

    #[test]
    fn reopens_from_disk() {
        let dir = std::env::temp_dir().join(format!("relay-gym-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gym.db");
        let text = include_str!("../tests/avex_export.json");
        let h = crate::export::read(text, "avex_export.json", "t").unwrap();
        Gym::open(&path).unwrap().replace(&h).unwrap();
        assert_eq!(Gym::open(&path).unwrap().load().unwrap(), h);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
