//! File-based store. In-memory state is keyed by id (HashMap) and persisted
//! atomically to JSON on every write. The on-disk shape is a JSON array per
//! collection so the format stays straightforward and human-inspectable:
//!
//!   <data>/projects.json     — `[ProjectRow, ...]`
//!   <data>/iterations.json   — `[IterationRow, ...]`
//!   <data>/tasks.json
//!   <data>/steering.json
//!   <data>/questions.json
//!   <data>/chat.json
//!   <data>/events/<project_id>.jsonl  — append-only event log

use crate::error::AppResult;
use crate::events::EventPayload;
use crate::types::*;
use crate::util::RwLockExt;
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub struct Store {
    data_dir: PathBuf,
    inner: RwLock<Inner>,
}

/// In-memory collections, all keyed by primary id. Iteration / filtering by
/// secondary keys (project_id, iteration_id) is done by scanning `.values()`
/// — fine for the volumes this app handles (hundreds of rows, not millions).
#[derive(Default)]
struct Inner {
    projects: HashMap<String, ProjectRow>,
    iterations: HashMap<String, IterationRow>,
    tasks: HashMap<String, TaskRow>,
    steering: HashMap<String, SteeringRow>,
    questions: HashMap<String, QuestionRow>,
    chat: HashMap<String, ChatMessageRow>,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn nano(len: usize) -> String {
    nanoid::nanoid!(len)
}

/// Load a JSON array file into a HashMap keyed by `id`. Missing file → empty.
/// Corrupted file → empty plus a warning, so the app keeps starting instead
/// of getting stuck behind a single bad row.
fn load_collection<T>(path: &Path, key_of: impl Fn(&T) -> &str) -> HashMap<String, T>
where
    T: DeserializeOwned,
{
    if !path.exists() {
        return HashMap::new();
    }
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(?path, "store: cannot read file ({e}) — using defaults");
            return HashMap::new();
        }
    };
    let vec: Vec<T> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(?path, "store: cannot parse JSON ({e}) — using defaults");
            return HashMap::new();
        }
    };
    vec.into_iter()
        .map(|row| (key_of(&row).to_string(), row))
        .collect()
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}.json", std::process::id()));
    let mut f = fs::File::create(&tmp)?;
    let json = serde_json::to_string_pretty(value)?;
    f.write_all(json.as_bytes())?;
    f.sync_all().ok();
    fs::rename(&tmp, path)?;
    Ok(())
}

impl Store {
    pub fn open(data_dir: PathBuf) -> AppResult<Self> {
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(data_dir.join("events"))?;
        let projects = load_collection(&data_dir.join("projects.json"), |p: &ProjectRow| &p.id);
        let iterations =
            load_collection(&data_dir.join("iterations.json"), |i: &IterationRow| &i.id);
        let tasks = load_collection(&data_dir.join("tasks.json"), |t: &TaskRow| &t.id);
        let steering = load_collection(&data_dir.join("steering.json"), |s: &SteeringRow| &s.id);
        let questions =
            load_collection(&data_dir.join("questions.json"), |q: &QuestionRow| &q.id);
        let chat = load_collection(&data_dir.join("chat.json"), |c: &ChatMessageRow| &c.id);
        Ok(Self {
            data_dir,
            inner: RwLock::new(Inner {
                projects,
                iterations,
                tasks,
                steering,
                questions,
                chat,
            }),
        })
    }

    // ---- Projects ----
    pub fn create_project(&self, input: CreateProjectInput) -> AppResult<ProjectRow> {
        let row = ProjectRow {
            id: nano(10),
            name: input.name,
            idea: input.idea,
            root_path: input.root_path,
            state: ConductorState::Idle,
            created_at: now_ms(),
            // Defaults are CLI aliases — they automatically resolve to the
            // latest model of each family, so a project created today is
            // still using "the current Opus" a year from now without any
            // code changes.
            model_pm: input.model_pm.unwrap_or_else(|| "opus".into()),
            model_specialist: input.model_specialist.unwrap_or_else(|| "sonnet".into()),
            permission_mode: input
                .permission_mode
                .unwrap_or(PermissionMode::BypassPermissions),
        };
        self.inner
            .write_or_poisoned()
            .projects
            .insert(row.id.clone(), row.clone());
        self.flush_projects()?;
        Ok(row)
    }

    pub fn list_projects(&self) -> Vec<ProjectRow> {
        let g = self.inner.read_or_poisoned();
        let mut out: Vec<ProjectRow> = g.projects.values().cloned().collect();
        out.sort_by_key(|p| -p.created_at);
        out
    }

    pub fn get_project(&self, id: &str) -> Option<ProjectRow> {
        self.inner.read_or_poisoned().projects.get(id).cloned()
    }

    pub fn set_project_state(&self, id: &str, state: ConductorState) -> AppResult<()> {
        if let Some(p) = self.inner.write_or_poisoned().projects.get_mut(id) {
            p.state = state;
        }
        self.flush_projects()
    }

    pub fn rename_project(&self, id: &str, name: &str, idea: &str) -> AppResult<()> {
        if let Some(p) = self.inner.write_or_poisoned().projects.get_mut(id) {
            p.name = name.to_string();
            p.idea = idea.to_string();
        }
        self.flush_projects()
    }

    /// Cascade-delete a project and everything attached to it in one lock
    /// acquisition. The on-disk event log is removed too.
    pub fn delete_project(&self, id: &str) -> AppResult<()> {
        {
            let mut g = self.inner.write_or_poisoned();
            g.projects.remove(id);
            let removed_iters: HashSet<String> = g
                .iterations
                .values()
                .filter(|i| i.project_id == id)
                .map(|i| i.id.clone())
                .collect();
            g.iterations.retain(|_, i| i.project_id != id);
            g.tasks.retain(|_, t| !removed_iters.contains(&t.iteration_id));
            g.steering.retain(|_, s| s.project_id != id);
            g.questions.retain(|_, q| q.project_id != id);
            g.chat.retain(|_, c| c.project_id != id);
        }
        self.flush_all()?;
        let ev = self.data_dir.join("events").join(format!("{id}.jsonl"));
        if ev.exists() {
            if let Err(e) = fs::remove_file(&ev) {
                tracing::warn!(?ev, "store: failed to remove event log: {e}");
            }
        }
        Ok(())
    }

    /// On app startup nothing is running yet — drop any lingering "in flight"
    /// statuses to a recoverable state. Returns `(iters_reset, tasks_reset,
    /// questions_cancelled)`. The iter counter is reserved for future use.
    pub fn reset_stale_states(&self) -> AppResult<(usize, usize, usize)> {
        let mut tasks_changed = 0usize;
        let mut questions_changed = 0usize;
        {
            let mut g = self.inner.write_or_poisoned();
            for p in g.projects.values_mut() {
                use ConductorState::*;
                if matches!(
                    p.state,
                    Running | WrappingUp | PreparingPreview | Resuming | Presenting
                ) {
                    p.state = Idle;
                }
            }
            let active_iters: HashSet<String> = g
                .iterations
                .values()
                .filter(|i| {
                    matches!(
                        i.status,
                        IterationStatus::Running | IterationStatus::WrappingUp
                    )
                })
                .map(|i| i.id.clone())
                .collect();
            for t in g.tasks.values_mut() {
                let needs_reset = matches!(t.status, TaskStatus::InProgress)
                    || (matches!(t.status, TaskStatus::Failed | TaskStatus::Skipped)
                        && active_iters.contains(&t.iteration_id));
                if needs_reset {
                    t.status = TaskStatus::Pending;
                    t.started_at = None;
                    t.ended_at = None;
                    tasks_changed += 1;
                }
            }
            for q in g.questions.values_mut() {
                if matches!(q.status, QuestionStatus::Pending) {
                    q.status = QuestionStatus::Cancelled;
                    q.answered_at = Some(now_ms());
                    questions_changed += 1;
                }
            }
        }
        self.flush_all()?;
        Ok((0, tasks_changed, questions_changed))
    }

    // ---- Iterations ----
    pub fn create_iteration(&self, project_id: &str) -> AppResult<IterationRow> {
        let row = {
            let mut g = self.inner.write_or_poisoned();
            let number = g
                .iterations
                .values()
                .filter(|i| i.project_id == project_id)
                .map(|i| i.number)
                .max()
                .unwrap_or(0)
                + 1;
            let row = IterationRow {
                id: nano(10),
                project_id: project_id.to_string(),
                number,
                status: IterationStatus::Running,
                started_at: now_ms(),
                ended_at: None,
                summary: None,
                theme: None,
                rationale: None,
                stories: vec![],
                stack_notes: None,
                mode: None,
            };
            g.iterations.insert(row.id.clone(), row.clone());
            row
        };
        self.flush_iterations()?;
        Ok(row)
    }

    pub fn set_iteration_status(
        &self,
        id: &str,
        status: IterationStatus,
        summary: Option<&str>,
    ) -> AppResult<()> {
        if let Some(it) = self.inner.write_or_poisoned().iterations.get_mut(id) {
            it.status = status;
            if let Some(s) = summary {
                it.summary = Some(s.to_string());
            }
            if matches!(
                status,
                IterationStatus::Completed | IterationStatus::Failed | IterationStatus::Presented
            ) {
                it.ended_at = Some(now_ms());
            }
        }
        self.flush_iterations()
    }

    pub fn current_iteration(&self, project_id: &str) -> Option<IterationRow> {
        self.inner
            .read_or_poisoned()
            .iterations
            .values()
            .filter(|i| i.project_id == project_id)
            .max_by_key(|i| i.number)
            .cloned()
    }

    pub fn iterations_by_project(&self, project_id: &str) -> Vec<IterationRow> {
        let mut list: Vec<IterationRow> = self
            .inner
            .read_or_poisoned()
            .iterations
            .values()
            .filter(|i| i.project_id == project_id)
            .cloned()
            .collect();
        list.sort_by_key(|i| -i.number);
        list
    }

    pub fn find_resumable_iteration(&self, project_id: &str) -> Option<IterationRow> {
        self.iterations_by_project(project_id)
            .into_iter()
            .next()
            .filter(|i| {
                matches!(
                    i.status,
                    IterationStatus::Running | IterationStatus::WrappingUp
                )
            })
    }

    pub fn recent_iterations(&self, project_id: &str, n: usize) -> Vec<IterationRow> {
        self.iterations_by_project(project_id)
            .into_iter()
            .take(n)
            .collect()
    }

    pub fn set_iteration_meta(
        &self,
        id: &str,
        theme: Option<String>,
        rationale: Option<String>,
        stories: Option<Vec<IterationStory>>,
        stack_notes: Option<String>,
        mode: Option<IterationMode>,
    ) -> AppResult<()> {
        if let Some(it) = self.inner.write_or_poisoned().iterations.get_mut(id) {
            if let Some(v) = theme {
                it.theme = Some(v);
            }
            if let Some(v) = rationale {
                it.rationale = Some(v);
            }
            if let Some(v) = stories {
                it.stories = v;
            }
            if let Some(v) = stack_notes {
                it.stack_notes = Some(v);
            }
            if let Some(v) = mode {
                it.mode = Some(v);
            }
        }
        self.flush_iterations()
    }

    // ---- Tasks ----
    pub fn create_task(
        &self,
        iteration_id: &str,
        role: AgentRole,
        title: String,
        description: String,
        architect_id: Option<String>,
        depends_on: Vec<String>,
    ) -> AppResult<TaskRow> {
        let row = TaskRow {
            id: nano(10),
            iteration_id: iteration_id.to_string(),
            role,
            title,
            description,
            status: TaskStatus::Pending,
            worktree_path: None,
            branch: None,
            created_at: now_ms(),
            started_at: None,
            ended_at: None,
            architect_id,
            depends_on,
        };
        self.inner
            .write_or_poisoned()
            .tasks
            .insert(row.id.clone(), row.clone());
        self.flush_tasks()?;
        Ok(row)
    }

    pub fn set_task_status(&self, id: &str, status: TaskStatus) -> AppResult<()> {
        if let Some(t) = self.inner.write_or_poisoned().tasks.get_mut(id) {
            t.status = status;
            // Stamp `started_at` on the first transition to InProgress (idempotent
            // — retries keep the original start time so the UI timer is stable).
            if matches!(status, TaskStatus::InProgress) && t.started_at.is_none() {
                t.started_at = Some(now_ms());
            }
            if matches!(
                status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Skipped
            ) {
                t.ended_at = Some(now_ms());
            }
        }
        self.flush_tasks()
    }

    pub fn iteration_tasks(&self, iteration_id: &str) -> Vec<TaskRow> {
        let mut list: Vec<TaskRow> = self
            .inner
            .read_or_poisoned()
            .tasks
            .values()
            .filter(|t| t.iteration_id == iteration_id)
            .cloned()
            .collect();
        list.sort_by_key(|t| t.created_at);
        list
    }

    // ---- Events (append-only jsonl) ----
    /// Persist a typed event and return the materialized row. Append-only;
    /// no in-memory cache.
    pub fn insert_event(
        &self,
        project_id: &str,
        payload: EventPayload,
        iteration_id: Option<String>,
        task_id: Option<String>,
    ) -> AppResult<EventRow> {
        let row = EventRow {
            id: nano(12),
            project_id: project_id.to_string(),
            iteration_id,
            task_id,
            agent_role: payload.agent_role(),
            payload,
            ts: now_ms(),
        };
        // Parent dir is created once in `Store::open`; nothing to do here.
        let path = self.events_path(project_id);
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let line = serde_json::to_string(&row)?;
        writeln!(f, "{line}")?;
        Ok(row)
    }

    pub fn recent_events(&self, project_id: &str, limit: usize, since_ts: i64) -> Vec<EventRow> {
        let path = self.events_path(project_id);
        let Ok(content) = fs::read_to_string(&path) else {
            return Vec::new();
        };
        let mut out: Vec<EventRow> = Vec::with_capacity(limit);
        for line in content.lines().rev() {
            if line.is_empty() {
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<EventRow>(line) {
                if ev.ts > since_ts {
                    out.push(ev);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
        out
    }

    fn events_path(&self, project_id: &str) -> PathBuf {
        self.data_dir
            .join("events")
            .join(format!("{project_id}.jsonl"))
    }

    // ---- Steering ----
    pub fn push_steering(
        &self,
        project_id: &str,
        message: &str,
        mode: SteeringMode,
    ) -> AppResult<SteeringRow> {
        let row = SteeringRow {
            id: nano(10),
            project_id: project_id.to_string(),
            message: message.to_string(),
            mode,
            applied_iteration_id: None,
            ts: now_ms(),
        };
        self.inner
            .write_or_poisoned()
            .steering
            .insert(row.id.clone(), row.clone());
        self.flush_steering()?;
        Ok(row)
    }

    pub fn pending_steering(&self, project_id: &str) -> Option<SteeringRow> {
        self.inner
            .read_or_poisoned()
            .steering
            .values()
            .filter(|s| s.project_id == project_id && s.applied_iteration_id.is_none())
            .max_by_key(|s| s.ts)
            .cloned()
    }

    pub fn consume_steering(&self, id: &str, iteration_id: &str) -> AppResult<()> {
        if let Some(s) = self.inner.write_or_poisoned().steering.get_mut(id) {
            s.applied_iteration_id = Some(iteration_id.to_string());
        }
        self.flush_steering()
    }

    // ---- Questions ----
    pub fn push_question(
        &self,
        project_id: &str,
        iteration_id: Option<String>,
        task_id: Option<String>,
        agent_role: Option<AgentRole>,
        question: String,
        context: String,
    ) -> AppResult<QuestionRow> {
        let row = QuestionRow {
            id: nano(10),
            project_id: project_id.to_string(),
            iteration_id,
            task_id,
            agent_role,
            question,
            context,
            status: QuestionStatus::Pending,
            resolution: None,
            answer: None,
            created_at: now_ms(),
            answered_at: None,
        };
        self.inner
            .write_or_poisoned()
            .questions
            .insert(row.id.clone(), row.clone());
        self.flush_questions()?;
        Ok(row)
    }

    pub fn pending_questions(&self, project_id: &str) -> Vec<QuestionRow> {
        let mut list: Vec<QuestionRow> = self
            .inner
            .read_or_poisoned()
            .questions
            .values()
            .filter(|q| q.project_id == project_id && matches!(q.status, QuestionStatus::Pending))
            .cloned()
            .collect();
        list.sort_by_key(|q| q.created_at);
        list
    }

    pub fn get_question(&self, id: &str) -> Option<QuestionRow> {
        self.inner.read_or_poisoned().questions.get(id).cloned()
    }

    pub fn resolve_question(
        &self,
        id: &str,
        resolution: QuestionResolution,
        answer: String,
        auto: bool,
    ) -> AppResult<()> {
        if let Some(q) = self.inner.write_or_poisoned().questions.get_mut(id) {
            q.status = if auto {
                QuestionStatus::AutoAnswered
            } else {
                QuestionStatus::Answered
            };
            q.resolution = Some(resolution);
            q.answer = Some(answer);
            q.answered_at = Some(now_ms());
        }
        self.flush_questions()
    }

    pub fn cancel_question(&self, id: &str) -> AppResult<()> {
        if let Some(q) = self.inner.write_or_poisoned().questions.get_mut(id) {
            if matches!(q.status, QuestionStatus::Pending) {
                q.status = QuestionStatus::Cancelled;
                q.answered_at = Some(now_ms());
            }
        }
        self.flush_questions()
    }

    // ---- Chat ----
    pub fn push_chat(
        &self,
        project_id: &str,
        role: ChatRole,
        text: String,
        error: Option<String>,
    ) -> AppResult<ChatMessageRow> {
        let row = ChatMessageRow {
            id: nano(10),
            project_id: project_id.to_string(),
            role,
            text,
            ts: now_ms(),
            error,
        };
        self.inner
            .write_or_poisoned()
            .chat
            .insert(row.id.clone(), row.clone());
        self.flush_chat()?;
        Ok(row)
    }

    pub fn chat_history(&self, project_id: &str) -> Vec<ChatMessageRow> {
        let mut list: Vec<ChatMessageRow> = self
            .inner
            .read_or_poisoned()
            .chat
            .values()
            .filter(|c| c.project_id == project_id)
            .cloned()
            .collect();
        list.sort_by_key(|c| c.ts);
        list
    }

    // ---- flush helpers ----
    //
    // Each helper takes a snapshot of one collection's values and writes them
    // out as a JSON array. The on-disk format is intentionally array-shaped
    // so the files stay easy to inspect by hand.
    fn flush_all(&self) -> AppResult<()> {
        self.flush_projects()?;
        self.flush_iterations()?;
        self.flush_tasks()?;
        self.flush_steering()?;
        self.flush_questions()?;
        self.flush_chat()
    }

    fn flush_projects(&self) -> AppResult<()> {
        let g = self.inner.read_or_poisoned();
        let vec: Vec<&ProjectRow> = g.projects.values().collect();
        write_json_atomic(&self.data_dir.join("projects.json"), &vec)
    }
    fn flush_iterations(&self) -> AppResult<()> {
        let g = self.inner.read_or_poisoned();
        let vec: Vec<&IterationRow> = g.iterations.values().collect();
        write_json_atomic(&self.data_dir.join("iterations.json"), &vec)
    }
    fn flush_tasks(&self) -> AppResult<()> {
        let g = self.inner.read_or_poisoned();
        let vec: Vec<&TaskRow> = g.tasks.values().collect();
        write_json_atomic(&self.data_dir.join("tasks.json"), &vec)
    }
    fn flush_steering(&self) -> AppResult<()> {
        let g = self.inner.read_or_poisoned();
        let vec: Vec<&SteeringRow> = g.steering.values().collect();
        write_json_atomic(&self.data_dir.join("steering.json"), &vec)
    }
    fn flush_questions(&self) -> AppResult<()> {
        let g = self.inner.read_or_poisoned();
        let vec: Vec<&QuestionRow> = g.questions.values().collect();
        write_json_atomic(&self.data_dir.join("questions.json"), &vec)
    }
    fn flush_chat(&self) -> AppResult<()> {
        let g = self.inner.read_or_poisoned();
        let vec: Vec<&ChatMessageRow> = g.chat.values().collect();
        write_json_atomic(&self.data_dir.join("chat.json"), &vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (Store, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let s = Store::open(tmp.path().to_path_buf()).expect("open");
        (s, tmp)
    }

    fn create(s: &Store, name: &str) -> ProjectRow {
        s.create_project(CreateProjectInput {
            name: name.into(),
            idea: format!("test idea for {name}"),
            root_path: "/tmp/whatever".into(),
            model_pm: None,
            model_specialist: None,
            permission_mode: None,
        })
        .expect("create")
    }

    #[test]
    fn create_and_get_project() {
        let (s, _t) = store();
        let p = create(&s, "alpha");
        assert_eq!(s.get_project(&p.id).map(|p| p.name), Some("alpha".into()));
    }

    #[test]
    fn list_projects_sorted_by_created_at_desc() {
        let (s, _t) = store();
        let a = create(&s, "a");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = create(&s, "b");
        let list = s.list_projects();
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
    }

    #[test]
    fn delete_project_cascades() {
        let (s, _t) = store();
        let p = create(&s, "p");
        let it = s.create_iteration(&p.id).expect("iter");
        let t = s
            .create_task(
                &it.id,
                AgentRole::SpecialistBackend,
                "do x".into(),
                "details".into(),
                None,
                vec![],
            )
            .expect("task");
        s.push_steering(&p.id, "msg", SteeringMode::Soft).unwrap();
        s.push_question(&p.id, Some(it.id.clone()), Some(t.id.clone()), None, "?".into(), "ctx".into())
            .unwrap();
        s.push_chat(&p.id, ChatRole::User, "hi".into(), None).unwrap();

        s.delete_project(&p.id).expect("delete");

        assert!(s.get_project(&p.id).is_none());
        assert!(s.iterations_by_project(&p.id).is_empty());
        assert!(s.iteration_tasks(&it.id).is_empty());
        assert!(s.pending_steering(&p.id).is_none());
        assert!(s.pending_questions(&p.id).is_empty());
        assert!(s.chat_history(&p.id).is_empty());
    }

    #[test]
    fn iteration_numbers_increment_per_project() {
        let (s, _t) = store();
        let p = create(&s, "p");
        let a = s.create_iteration(&p.id).unwrap();
        let b = s.create_iteration(&p.id).unwrap();
        assert_eq!(a.number, 1);
        assert_eq!(b.number, 2);
    }

    #[test]
    fn reset_stale_states_clears_in_progress() {
        let (s, _t) = store();
        let p = create(&s, "p");
        let it = s.create_iteration(&p.id).unwrap();
        let task = s
            .create_task(
                &it.id,
                AgentRole::SpecialistBackend,
                "x".into(),
                "y".into(),
                None,
                vec![],
            )
            .unwrap();
        s.set_task_status(&task.id, TaskStatus::InProgress).unwrap();
        s.set_project_state(&p.id, ConductorState::Running).unwrap();

        let (_, tasks_reset, _) = s.reset_stale_states().unwrap();
        assert_eq!(tasks_reset, 1);
        let after = s.iteration_tasks(&it.id);
        assert!(matches!(after[0].status, TaskStatus::Pending));
        assert!(matches!(
            s.get_project(&p.id).unwrap().state,
            ConductorState::Idle
        ));
    }

    #[test]
    fn pending_steering_returns_latest_unapplied() {
        let (s, _t) = store();
        let p = create(&s, "p");
        let _first = s.push_steering(&p.id, "first", SteeringMode::Soft).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = s.push_steering(&p.id, "second", SteeringMode::Soft).unwrap();
        assert_eq!(s.pending_steering(&p.id).map(|s| s.id), Some(second.id.clone()));
        let it = s.create_iteration(&p.id).unwrap();
        s.consume_steering(&second.id, &it.id).unwrap();
        // after consuming, falls back to `first` which is older but still pending
        let next = s.pending_steering(&p.id).unwrap();
        assert_eq!(next.message, "first");
    }
}
