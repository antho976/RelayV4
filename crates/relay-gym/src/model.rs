//! The training history as Avex's export carries it, one row per thing the person did. Times are
//! epoch milliseconds, weights pounds, as Avex keeps them.

/// What the file said about itself and the person's settings, and when it reached the PC.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Meta {
    pub export_version: i64,
    /// As Avex wrote it: a local date and time on the phone.
    pub exported_at: String,
    pub app_version: String,
    /// RFC 3339, when the PC read the file.
    pub imported_at: String,
    /// The file's name on the PC.
    pub file: String,
    pub use_kg: bool,
    pub user_name: String,
    pub user_goal: String,
    pub days_per_week: i64,
    pub first_day_monday: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    pub meta: Meta,
    /// Oldest first.
    pub sessions: Vec<Session>,
    /// Oldest first.
    pub cardio: Vec<Cardio>,
    pub goals: Vec<Goal>,
}

/// One finished workout.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    /// Avex's own id for it.
    pub id: i64,
    /// The program day it was, like `push_a`; empty for freestyle.
    pub day_key: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// Time actually training, without the away time of a session resumed later.
    pub active_seconds: i64,
    /// Avex's own total, which its screens show.
    pub volume_lb: f64,
    pub pr_count: i64,
    pub set_count: i64,
    pub session_type: String,
    pub intensity: String,
    /// Excluded from Avex's stats by the person; kept, shown, and left out of totals here too.
    pub untracked: bool,
    pub tags: String,
    pub journal: String,
    pub mood: String,
    pub exercises: Vec<Exercise>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Exercise {
    /// Avex's catalogue id, or a custom one.
    pub exercise_id: String,
    /// The name Avex shows, which is what lifts are grouped by.
    pub name: String,
    pub position: i64,
    pub difficulty: String,
    pub skipped: bool,
    pub note: String,
    /// Avex marked a personal record on it.
    pub was_pr: bool,
    pub hit_full_target: bool,
    pub superset: String,
    pub sets: Vec<Set>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Set {
    /// Absent for a bodyweight set.
    pub weight_lb: Option<f64>,
    /// What the person typed, already canonical pounds.
    pub weight_text: String,
    pub reps: i64,
    pub rpe: Option<f64>,
    pub completed_at: Option<i64>,
    /// `warmup`, `drop` and the like; empty for a working set.
    pub set_type: String,
    pub difficulty_tag: String,
    pub duration_seconds: Option<i64>,
    pub assisted: bool,
    pub amrap: bool,
    pub to_failure: bool,
}

impl Set {
    pub fn warmup(&self) -> bool {
        self.set_type.eq_ignore_ascii_case("warmup")
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cardio {
    pub date: i64,
    /// `run`, `bike`, `rest`…, as Avex names them.
    pub kind: String,
    pub duration_min: i64,
    pub distance_km: Option<f64>,
    pub effort: String,
    pub rest_reason: String,
    pub note: String,
    pub interval_count: Option<i64>,
    pub hr_zone: String,
    pub incline_pct: Option<f64>,
    pub laps: Option<i64>,
    pub elevation_m: Option<f64>,
    pub conditions: String,
}

/// A goal Avex's coach keeps.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Goal {
    pub kind: String,
    pub target_key: String,
    pub target_value: Option<f64>,
    pub priority: i64,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub archived_at: Option<i64>,
    pub source: String,
    pub note: String,
}
