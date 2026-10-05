//! Session persistence in `kara.db` in Kara's data directory (SQLite).
//!
//! Stores session metadata, per-turn outcomes, and the agent's resumable
//! state (task state and compact conversation history). Undo snapshots live
//! in `sessions/<id>/` in Kara's data directory.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub workspace: String,
    pub created: String,
    pub updated: String,
    pub model: String,
    pub title: String,
    pub turns: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    pub turn: u64,
    pub task: String,
    pub mode: String,
    pub outcome: String,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub started: String,
    pub finished: String,
}

pub struct SessionStore {
    db: PathBuf,
}

impl SessionStore {
    pub fn open(db: &Path) -> anyhow::Result<SessionStore> {
        if let Some(p) = db.parent() {
            std::fs::create_dir_all(p)?;
        }
        let s = SessionStore {
            db: db.to_path_buf(),
        };
        s.conn()?.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY, workspace TEXT NOT NULL, created TEXT NOT NULL,
                updated TEXT NOT NULL, model TEXT NOT NULL DEFAULT '', title TEXT NOT NULL DEFAULT '');
             CREATE INDEX IF NOT EXISTS sessions_ws ON sessions(workspace, updated);
             CREATE TABLE IF NOT EXISTS turns (
                session_id TEXT NOT NULL, turn INTEGER NOT NULL, task TEXT NOT NULL, mode TEXT NOT NULL,
                outcome TEXT NOT NULL, summary TEXT NOT NULL, changed_files TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL, completion_tokens INTEGER NOT NULL,
                started TEXT NOT NULL, finished TEXT NOT NULL,
                PRIMARY KEY (session_id, turn));
             CREATE TABLE IF NOT EXISTS agent_state (
                session_id TEXT PRIMARY KEY, state TEXT NOT NULL, history TEXT NOT NULL);",
        )?;
        Ok(s)
    }

    fn conn(&self) -> anyhow::Result<Connection> {
        let c = Connection::open(&self.db)?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        c.pragma_update(None, "journal_mode", "WAL")?;
        Ok(c)
    }

    pub fn create(&self, workspace: &Path, model: &str) -> anyhow::Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn()?.execute(
            "INSERT INTO sessions(id, workspace, created, updated, model, title) VALUES (?1, ?2, ?3, ?3, ?4, '')",
            params![id, workspace.to_string_lossy(), now, model],
        )?;
        Ok(id)
    }

    pub fn latest_for(&self, workspace: &Path) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn()?
            .query_row(
                "SELECT id FROM sessions WHERE workspace = ?1 ORDER BY updated DESC LIMIT 1",
                [workspace.to_string_lossy()],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn list(&self, workspace: Option<&Path>, limit: usize) -> anyhow::Result<Vec<SessionInfo>> {
        let c = self.conn()?;
        let mut st = c.prepare(
            "SELECT s.id, s.workspace, s.created, s.updated, s.model, s.title,
                    (SELECT COUNT(*) FROM turns t WHERE t.session_id = s.id)
             FROM sessions s WHERE (?1 IS NULL OR s.workspace = ?1)
             ORDER BY s.updated DESC LIMIT ?2",
        )?;
        let ws = workspace.map(|w| w.to_string_lossy().into_owned());
        let rows = st.query_map(params![ws, limit as i64], |r| {
            Ok(SessionInfo {
                id: r.get(0)?,
                workspace: r.get(1)?,
                created: r.get(2)?,
                updated: r.get(3)?,
                model: r.get(4)?,
                title: r.get(5)?,
                turns: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn record_turn(&self, session: &str, t: &TurnRecord) -> anyhow::Result<()> {
        let c = self.conn()?;
        c.execute(
            "INSERT OR REPLACE INTO turns VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                session,
                t.turn as i64,
                t.task,
                t.mode,
                t.outcome,
                t.summary,
                serde_json::to_string(&t.changed_files)?,
                t.prompt_tokens as i64,
                t.completion_tokens as i64,
                t.started,
                t.finished
            ],
        )?;
        let title: String = t.task.chars().take(80).collect();
        c.execute(
            "UPDATE sessions SET updated = ?2, title = CASE WHEN title = '' THEN ?3 ELSE title END WHERE id = ?1",
            params![session, t.finished, title],
        )?;
        Ok(())
    }

    pub fn turns(&self, session: &str) -> anyhow::Result<Vec<TurnRecord>> {
        let c = self.conn()?;
        let mut st = c.prepare(
            "SELECT turn, task, mode, outcome, summary, changed_files, prompt_tokens, completion_tokens, started, finished
             FROM turns WHERE session_id = ?1 ORDER BY turn",
        )?;
        let rows = st.query_map([session], |r| {
            let files: String = r.get(5)?;
            Ok(TurnRecord {
                turn: r.get::<_, i64>(0)? as u64,
                task: r.get(1)?,
                mode: r.get(2)?,
                outcome: r.get(3)?,
                summary: r.get(4)?,
                changed_files: serde_json::from_str(&files).unwrap_or_default(),
                prompt_tokens: r.get::<_, i64>(6)? as u64,
                completion_tokens: r.get::<_, i64>(7)? as u64,
                started: r.get(8)?,
                finished: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn save_state(
        &self,
        session: &str,
        state: &serde_json::Value,
        history: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT OR REPLACE INTO agent_state(session_id, state, history) VALUES (?1, ?2, ?3)",
            params![session, state.to_string(), history.to_string()],
        )?;
        Ok(())
    }

    pub fn load_state(
        &self,
        session: &str,
    ) -> anyhow::Result<Option<(serde_json::Value, serde_json::Value)>> {
        let row: Option<(String, String)> = self
            .conn()?
            .query_row(
                "SELECT state, history FROM agent_state WHERE session_id = ?1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((s, h)) => Some((serde_json::from_str(&s)?, serde_json::from_str(&h)?)),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("kara.db")).unwrap();
        let ws = dir.path().join("repo");
        let id = store.create(&ws, "test-model").unwrap();
        assert_eq!(store.latest_for(&ws).unwrap().as_deref(), Some(id.as_str()));
        store
            .record_turn(
                &id,
                &TurnRecord {
                    turn: 1,
                    task: "fix auth".into(),
                    mode: "execute".into(),
                    outcome: "completed".into(),
                    summary: "done".into(),
                    changed_files: vec!["a.rb".into()],
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    started: "s".into(),
                    finished: "f".into(),
                },
            )
            .unwrap();
        let list = store.list(Some(&ws), 10).unwrap();
        assert_eq!(list[0].title, "fix auth");
        assert_eq!(list[0].turns, 1);
        assert_eq!(store.turns(&id).unwrap()[0].changed_files, vec!["a.rb"]);
        store
            .save_state(&id, &serde_json::json!({"a": 1}), &serde_json::json!([]))
            .unwrap();
        assert_eq!(store.load_state(&id).unwrap().unwrap().0["a"], 1);
    }
}
