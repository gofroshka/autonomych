//! File-based JSON store. Same layout as the TS implementation:
//!   <data>/projects.json
//!   <data>/iterations.json
//!   <data>/tasks.json
//!   <data>/steering.json
//!   <data>/questions.json
//!   <data>/chat.json
//!   <data>/events/<project_id>.jsonl

use crate::error::{AppError, AppResult};
use crate::types::*;
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Store {
    pub data_dir: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    projects: Vec<ProjectRow>,
    iterations: Vec<IterationRow>,
    tasks: Vec<TaskRow>,
    steering: Vec<SteeringRow>,
    questions: Vec<QuestionRow>,
    chat: Vec<ChatMessageRow>,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn nano(len: usize) -> String {
    nanoid::nanoid!(len)
}

fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    if !path.exists() {
        return T::default();
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp-{}.json",
        std::process::id()
    ));
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
        let projects = load_json::<Vec<ProjectRow>>(&data_dir.join("projects.json"));
        let iterations = load_json::<Vec<IterationRow>>(&data_dir.join("iterations.json"));
        let tasks = load_json::<Vec<TaskRow>>(&data_dir.join("tasks.json"));
        let steering = load_json::<Vec<SteeringRow>>(&data_dir.join("steering.json"));
        let questions = load_json::<Vec<QuestionRow>>(&data_dir.join("questions.json"));
        let chat = load_json::<Vec<ChatMessageRow>>(&data_dir.join("chat.json"));
        Ok(Self {
            data_dir,
            inner: Mutex::new(Inner {
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
            model_pm: input.model_pm.unwrap_or_else(|| "claude-opus-4-5".into()),
            model_specialist: input
                .model_specialist
                .unwrap_or_else(|| "claude-sonnet-4-5".into()),
            permission_mode: input
                .permission_mode
                .unwrap_or(PermissionMode::BypassPermissions),
        };
        {
            let mut g = self.inner.lock().unwrap();
            g.projects.push(row.clone());
        }
        self.flush_projects()?;
        Ok(row)
    }

    pub fn list_projects(&self) -> Vec<ProjectRow> {
        let g = self.inner.lock().unwrap();
        let mut out = g.projects.clone();
        out.sort_by_key(|p| -p.created_at);
        out
    }

    pub fn get_project(&self, id: &str) -> Option<ProjectRow> {
        self.inner
            .lock()
            .unwrap()
            .projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub fn set_project_state(&self, id: &str, state: ConductorState) -> AppResult<()> {
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(p) = g.projects.iter_mut().find(|p| p.id == id) {
                p.state = state;
            }
        }
        self.flush_projects()
    }

    pub fn rename_project(&self, id: &str, name: &str, idea: &str) -> AppResult<()> {
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(p) = g.projects.iter_mut().find(|p| p.id == id) {
                p.name = name.to_string();
                p.idea = idea.to_string();
            }
        }
        self.flush_projects()
    }

    pub fn delete_project(&self, id: &str) -> AppResult<()> {
        {
            let mut g = self.inner.lock().unwrap();
            g.projects.retain(|p| p.id != id);
            let removed_iters: HashSet<String> = g
                .iterations
                .iter()
                .filter(|i| i.project_id == id)
                .map(|i| i.id.clone())
                .collect();
            g.iterations.retain(|i| i.project_id != id);
            g.tasks.retain(|t| !removed_iters.contains(&t.iteration_id));
            g.steering.retain(|s| s.project_id != id);
            g.questions.retain(|q| q.project_id != id);
            g.chat.retain(|c| c.project_id != id);
        }
        self.flush_all()?;
        let ev = self.data_dir.join("events").join(format!("{id}.jsonl"));
        if ev.exists() {
            fs::remove_file(ev).ok();
        }
        Ok(())
    }

    /// On app startup nothing is running yet. Mirror of the TS reaper.
    pub fn reset_stale_states(&self) -> AppResult<(usize, usize, usize)> {
        let mut tasks_changed = 0;
        let mut iters_changed = 0;
        let mut questions_changed = 0;
        {
            let mut g = self.inner.lock().unwrap();
            for p in g.projects.iter_mut() {
                use ConductorState::*;
                if matches!(p.state, Running | WrappingUp | PreparingPreview | Resuming | Presenting) {
                    p.state = Idle;
                }
            }
            let active_iters: HashSet<String> = g
                .iterations
                .iter()
                .filter(|i| matches!(i.status, IterationStatus::Running | IterationStatus::WrappingUp))
                .map(|i| i.id.clone())
                .collect();
            for t in g.tasks.iter_mut() {
                let needs_reset = matches!(t.status, TaskStatus::InProgress)
                    || (matches!(t.status, TaskStatus::Failed | TaskStatus::Skipped)
                        && active_iters.contains(&t.iteration_id));
                if needs_reset {
                    t.status = TaskStatus::Pending;
                    t.ended_at = None;
                    tasks_changed += 1;
                }
            }
            for q in g.questions.iter_mut() {
                if matches!(q.status, QuestionStatus::Pending) {
                    q.status = QuestionStatus::Cancelled;
                    q.answered_at = Some(now_ms());
                    questions_changed += 1;
                }
            }
            iters_changed = 0;
        }
        self.flush_all()?;
        Ok((iters_changed, tasks_changed, questions_changed))
    }

    // ---- Iterations ----
    pub fn create_iteration(&self, project_id: &str) -> AppResult<IterationRow> {
        let row = {
            let mut g = self.inner.lock().unwrap();
            let number = g
                .iterations
                .iter()
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
            g.iterations.push(row.clone());
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
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(it) = g.iterations.iter_mut().find(|i| i.id == id) {
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
        }
        self.flush_iterations()
    }

    pub fn current_iteration(&self, project_id: &str) -> Option<IterationRow> {
        let g = self.inner.lock().unwrap();
        g.iterations
            .iter()
            .filter(|i| i.project_id == project_id)
            .max_by_key(|i| i.number)
            .cloned()
    }

    pub fn iterations_by_project(&self, project_id: &str) -> Vec<IterationRow> {
        let g = self.inner.lock().unwrap();
        let mut list: Vec<IterationRow> = g
            .iterations
            .iter()
            .filter(|i| i.project_id == project_id)
            .cloned()
            .collect();
        list.sort_by_key(|i| -i.number);
        list
    }

    pub fn find_resumable_iteration(&self, project_id: &str) -> Option<IterationRow> {
        let list = self.iterations_by_project(project_id);
        list.into_iter().next().filter(|i| {
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
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(it) = g.iterations.iter_mut().find(|i| i.id == id) {
                if let Some(v) = theme { it.theme = Some(v); }
                if let Some(v) = rationale { it.rationale = Some(v); }
                if let Some(v) = stories { it.stories = v; }
                if let Some(v) = stack_notes { it.stack_notes = Some(v); }
                if let Some(v) = mode { it.mode = Some(v); }
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
            ended_at: None,
            architect_id,
            depends_on,
        };
        {
            let mut g = self.inner.lock().unwrap();
            g.tasks.push(row.clone());
        }
        self.flush_tasks()?;
        Ok(row)
    }

    pub fn set_task_status(&self, id: &str, status: TaskStatus) -> AppResult<()> {
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(t) = g.tasks.iter_mut().find(|t| t.id == id) {
                t.status = status;
                if matches!(
                    status,
                    TaskStatus::Done | TaskStatus::Failed | TaskStatus::Skipped
                ) {
                    t.ended_at = Some(now_ms());
                }
            }
        }
        self.flush_tasks()
    }

    pub fn iteration_tasks(&self, iteration_id: &str) -> Vec<TaskRow> {
        let g = self.inner.lock().unwrap();
        let mut list: Vec<TaskRow> = g
            .tasks
            .iter()
            .filter(|t| t.iteration_id == iteration_id)
            .cloned()
            .collect();
        list.sort_by_key(|t| t.created_at);
        list
    }

    // ---- Events (append-only jsonl) ----
    pub fn insert_event(
        &self,
        project_id: &str,
        r#type: EventType,
        payload: serde_json::Value,
        iteration_id: Option<String>,
        task_id: Option<String>,
        agent_role: Option<AgentRole>,
    ) -> AppResult<EventRow> {
        let row = EventRow {
            id: nano(12),
            project_id: project_id.to_string(),
            iteration_id,
            task_id,
            agent_role,
            r#type,
            payload: payload.to_string(),
            ts: now_ms(),
        };
        let path = self.events_path(project_id);
        fs::create_dir_all(path.parent().unwrap())?;
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
        self.data_dir.join("events").join(format!("{project_id}.jsonl"))
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
        {
            let mut g = self.inner.lock().unwrap();
            g.steering.push(row.clone());
        }
        self.flush_steering()?;
        Ok(row)
    }

    pub fn pending_steering(&self, project_id: &str) -> Option<SteeringRow> {
        let g = self.inner.lock().unwrap();
        g.steering
            .iter()
            .filter(|s| s.project_id == project_id && s.applied_iteration_id.is_none())
            .max_by_key(|s| s.ts)
            .cloned()
    }

    pub fn consume_steering(&self, id: &str, iteration_id: &str) -> AppResult<()> {
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(s) = g.steering.iter_mut().find(|s| s.id == id) {
                s.applied_iteration_id = Some(iteration_id.to_string());
            }
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
        {
            let mut g = self.inner.lock().unwrap();
            g.questions.push(row.clone());
        }
        self.flush_questions()?;
        Ok(row)
    }

    pub fn pending_questions(&self, project_id: &str) -> Vec<QuestionRow> {
        let g = self.inner.lock().unwrap();
        let mut list: Vec<QuestionRow> = g
            .questions
            .iter()
            .filter(|q| q.project_id == project_id && matches!(q.status, QuestionStatus::Pending))
            .cloned()
            .collect();
        list.sort_by_key(|q| q.created_at);
        list
    }

    pub fn get_question(&self, id: &str) -> Option<QuestionRow> {
        self.inner
            .lock()
            .unwrap()
            .questions
            .iter()
            .find(|q| q.id == id)
            .cloned()
    }

    pub fn resolve_question(
        &self,
        id: &str,
        resolution: QuestionResolution,
        answer: String,
        auto: bool,
    ) -> AppResult<()> {
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(q) = g.questions.iter_mut().find(|q| q.id == id) {
                q.status = if auto {
                    QuestionStatus::AutoAnswered
                } else {
                    QuestionStatus::Answered
                };
                q.resolution = Some(resolution);
                q.answer = Some(answer);
                q.answered_at = Some(now_ms());
            }
        }
        self.flush_questions()
    }

    pub fn cancel_question(&self, id: &str) -> AppResult<()> {
        {
            let mut g = self.inner.lock().unwrap();
            if let Some(q) = g.questions.iter_mut().find(|q| q.id == id) {
                if matches!(q.status, QuestionStatus::Pending) {
                    q.status = QuestionStatus::Cancelled;
                    q.answered_at = Some(now_ms());
                }
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
        {
            let mut g = self.inner.lock().unwrap();
            g.chat.push(row.clone());
        }
        self.flush_chat()?;
        Ok(row)
    }

    pub fn chat_history(&self, project_id: &str) -> Vec<ChatMessageRow> {
        let g = self.inner.lock().unwrap();
        let mut list: Vec<ChatMessageRow> = g
            .chat
            .iter()
            .filter(|c| c.project_id == project_id)
            .cloned()
            .collect();
        list.sort_by_key(|c| c.ts);
        list
    }

    // ---- flush helpers ----
    fn flush_all(&self) -> AppResult<()> {
        self.flush_projects()?;
        self.flush_iterations()?;
        self.flush_tasks()?;
        self.flush_steering()?;
        self.flush_questions()?;
        self.flush_chat()
    }

    fn flush_projects(&self) -> AppResult<()> {
        let g = self.inner.lock().unwrap();
        write_json_atomic(&self.data_dir.join("projects.json"), &g.projects)
    }
    fn flush_iterations(&self) -> AppResult<()> {
        let g = self.inner.lock().unwrap();
        write_json_atomic(&self.data_dir.join("iterations.json"), &g.iterations)
    }
    fn flush_tasks(&self) -> AppResult<()> {
        let g = self.inner.lock().unwrap();
        write_json_atomic(&self.data_dir.join("tasks.json"), &g.tasks)
    }
    fn flush_steering(&self) -> AppResult<()> {
        let g = self.inner.lock().unwrap();
        write_json_atomic(&self.data_dir.join("steering.json"), &g.steering)
    }
    fn flush_questions(&self) -> AppResult<()> {
        let g = self.inner.lock().unwrap();
        write_json_atomic(&self.data_dir.join("questions.json"), &g.questions)
    }
    fn flush_chat(&self) -> AppResult<()> {
        let g = self.inner.lock().unwrap();
        write_json_atomic(&self.data_dir.join("chat.json"), &g.chat)
    }
}

impl AppError {
    pub fn other(msg: impl Into<String>) -> Self {
        AppError::Other(msg.into())
    }
}
