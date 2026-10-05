use super::*;
use serde_json::{Value, json};

fn session_row(row: &libsql::Row) -> Result<ExecutionSession> {
    Ok(ExecutionSession {
        id: row
            .get::<String>(0)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        workspace_id: row
            .get::<String>(1)
            .map_err(storage_error)?
            .parse()
            .map_err(storage_error)?,
        label: row.get(2).map_err(storage_error)?,
        closed: row.get::<String>(3).map_err(storage_error)? == "closed",
        created_at: row.get::<i64>(4).map_err(storage_error)? as u64,
        updated_at: row.get::<i64>(5).map_err(storage_error)? as u64,
    })
}
const SESSION_COLUMNS: &str = "id,workspace_id,label,status,created_at,updated_at";

#[async_trait]
impl SessionRepository for LocalStore {
    async fn create_session(
        &self,
        workspace: WorkspaceId,
        id: ExecutionSessionId,
        label: &str,
    ) -> Result<ExecutionSession> {
        if label.len() > 1024 {
            return Err(PortError::Policy(
                "session label exceeds 1024 UTF-8 bytes".into(),
            ));
        }
        let label = label.to_owned();
        self.run(move|c|async move{
            let now=runtime_protocol::now_ms() as i64;
            c.execute("INSERT OR IGNORE INTO sessions(id,workspace_id,label,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",params![id.to_string(),workspace.to_string(),label.clone(),now]).await.map_err(storage_error)?;
            let mut rows=c.query(&format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"),[id.to_string()]).await.map_err(storage_error)?;
            let session=session_row(&rows.next().await.map_err(storage_error)?.ok_or(PortError::NotFound)?)?;
            if session.workspace_id!=workspace || session.label!=label {return Err(PortError::Conflict);}
            Ok(session)
        }).await
    }
    async fn session(
        &self,
        workspace: WorkspaceId,
        id: ExecutionSessionId,
    ) -> Result<ExecutionSession> {
        self.run(move |c| async move {
            let mut rows = c
                .query(
                    &format!(
                        "SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1 AND workspace_id=?2"
                    ),
                    params![id.to_string(), workspace.to_string()],
                )
                .await
                .map_err(storage_error)?;
            session_row(
                &rows
                    .next()
                    .await
                    .map_err(storage_error)?
                    .ok_or(PortError::NotFound)?,
            )
        })
        .await
    }
    async fn list_sessions(
        &self,
        workspace: WorkspaceId,
        before: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<ExecutionSession>, Option<u64>)> {
        let limit = limit.clamp(1, 100);
        let cursor = before.unwrap_or(i64::MAX as u64).min(i64::MAX as u64) as i64;
        self.run(move|c|async move{
            let mut rows=c.query(&format!("SELECT {SESSION_COLUMNS},seq FROM sessions WHERE workspace_id=?1 AND seq<?2 ORDER BY seq DESC LIMIT ?3"),params![workspace.to_string(),cursor,(limit+1) as i64]).await.map_err(storage_error)?;
            let mut sessions=Vec::new();let mut last=None;let mut more=false;
            while let Some(row)=rows.next().await.map_err(storage_error)?{
                if sessions.len()==limit{more=true;break;}
                last=Some(row.get::<i64>(6).map_err(storage_error)? as u64);
                sessions.push(session_row(&row)?);
            }
            Ok((sessions,if more{last}else{None}))
        }).await
    }
    async fn set_session_closed(
        &self,
        workspace: WorkspaceId,
        id: ExecutionSessionId,
        closed: bool,
    ) -> Result<ExecutionSession> {
        self.run(move |c| async move {
            let tx = c
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await
                .map_err(storage_error)?;
            let count = tx
                .execute(
                    "UPDATE sessions SET status=?3,updated_at=?4 WHERE id=?1 AND workspace_id=?2",
                    params![
                        id.to_string(),
                        workspace.to_string(),
                        if closed { "closed" } else { "open" },
                        runtime_protocol::now_ms() as i64
                    ],
                )
                .await
                .map_err(storage_error)?;
            if count != 1 {
                return Err(PortError::NotFound);
            }
            let mut rows = tx
                .query(
                    &format!(
                        "SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1 AND workspace_id=?2"
                    ),
                    params![id.to_string(), workspace.to_string()],
                )
                .await
                .map_err(storage_error)?;
            let session = session_row(
                &rows
                    .next()
                    .await
                    .map_err(storage_error)?
                    .ok_or(PortError::NotFound)?,
            )?;
            tx.commit().await.map_err(storage_error)?;
            Ok(session)
        })
        .await
    }
    async fn operation_session(
        &self,
        workspace: WorkspaceId,
        id: OperationId,
    ) -> Result<Option<ExecutionSessionId>> {
        self.run(move |c| async move {
            let mut rows = c
                .query(
                    "SELECT session_id FROM operations WHERE id=?1 AND workspace_id=?2",
                    params![id.to_string(), workspace.to_string()],
                )
                .await
                .map_err(storage_error)?;
            let session: Option<String> = rows
                .next()
                .await
                .map_err(storage_error)?
                .ok_or(PortError::NotFound)?
                .get(0)
                .map_err(storage_error)?;
            session
                .map(|s| s.parse().map_err(storage_error))
                .transpose()
        })
        .await
    }
}

pub struct HistorySearch {
    pub workspace: WorkspaceId,
    pub session: Option<ExecutionSessionId>,
    pub query: String,
    pub cursor: Option<u64>,
    pub limit: usize,
    pub budget_bytes: usize,
}
impl LocalStore {
    pub async fn session_notes(
        &self,
        workspace: WorkspaceId,
        session: ExecutionSessionId,
        cursor: Option<u64>,
        limit: usize,
        budget: usize,
    ) -> Result<Value> {
        self.session(workspace, session).await?;
        let cursor = cursor.unwrap_or(i64::MAX as u64).min(i64::MAX as u64);
        let limit = limit.clamp(1, 100);
        let budget = budget.min(128 * 1024);
        self.run(move|c|async move{
            let mut rows=c.query("SELECT e.id,e.operation_id,e.summary,e.created_at,e.seq FROM events e JOIN operations o ON o.id=e.operation_id WHERE e.workspace_id=?1 AND o.session_id=?2 AND e.type='decision' AND e.seq<?3 ORDER BY e.seq DESC LIMIT ?4",params![workspace.to_string(),session.to_string(),cursor as i64,(limit+1) as i64]).await.map_err(storage_error)?;
            let mut notes=Vec::new();let mut used=0;let mut last=cursor;let mut next=None;
            while let Some(row)=rows.next().await.map_err(storage_error)? {
                if notes.len()==limit {next=Some(last);break;}
                let id=row.get::<String>(0).map_err(storage_error)?;
                let note=json!({"event_id":id,"operation_id":row.get::<String>(1).map_err(storage_error)?,"text":row.get::<String>(2).map_err(storage_error)?,"created_at":row.get::<i64>(3).map_err(storage_error)?,"raw_ref":format!("context.event:{id}")});
                let bytes=serde_json::to_vec(&note).map_err(storage_error)?.len();
                if used+bytes>budget {next=Some(last);break;}
                used+=bytes;last=row.get::<i64>(4).map_err(storage_error)? as u64;notes.push(note);
            }
            Ok(json!({"session_id":session.to_string(),"notes":notes,"next_cursor":next.map(|n|n.to_string()),"bytes_used":used,"budget_bytes":budget}))
        }).await
    }
    /// Search the durable index, not the recent-context window. Payloads stay behind raw refs.
    pub async fn search_events(&self, request: HistorySearch) -> Result<Value> {
        if request.query.len() > 4096 {
            return Err(PortError::Policy(
                "history query exceeds 4096 UTF-8 bytes".into(),
            ));
        }
        if let Some(session) = request.session {
            self.session(request.workspace, session).await?;
        }
        let limit = request.limit.clamp(1, 1000);
        let budget = request.budget_bytes.min(128 * 1024);
        let cursor = request
            .cursor
            .unwrap_or(i64::MAX as u64)
            .min(i64::MAX as u64);
        self.run(move|c|async move{
            let mut rows=c.query("SELECT e.id,e.operation_id,e.type,e.summary,e.created_at,e.seq,op.session_id FROM events e JOIN operations op ON op.id=e.operation_id WHERE e.workspace_id=?1 AND e.seq<?2 AND (?3 IS NULL OR op.session_id=?3) AND (?4='' OR instr(lower(e.summary || ' ' || e.type || ' ' || op.tool),lower(?4))>0) ORDER BY e.seq DESC LIMIT ?5",params![request.workspace.to_string(),cursor as i64,request.session.map(|s|s.to_string()),request.query,(limit+1) as i64]).await.map_err(storage_error)?;
            let mut events=Vec::new();let mut used=0;let mut next=None;let mut last=cursor;
            while let Some(row)=rows.next().await.map_err(storage_error)?{
                if events.len()==limit {next=Some(last);break;}
                let event_id=row.get::<String>(0).map_err(storage_error)?;
                let event=json!({"event_id":event_id,"raw_ref":format!("context.event:{event_id}"),"operation_id":row.get::<String>(1).map_err(storage_error)?,"type":row.get::<String>(2).map_err(storage_error)?,"summary":row.get::<String>(3).map_err(storage_error)?,"created_at":row.get::<i64>(4).map_err(storage_error)?,"session_id":row.get::<Option<String>>(6).map_err(storage_error)?});
                let cost=serde_json::to_vec(&event).map_err(storage_error)?.len();
                if used+cost>budget {next=Some(last);break;}
                used+=cost;last=row.get::<i64>(5).map_err(storage_error)? as u64;
                events.push(event);
            }
            Ok(json!({"events":events,"next_cursor":next.map(|n|n.to_string()),"truncated":next.is_some(),"bytes_used":used,"budget_bytes":budget,"raw_payloads_included":false}))
        }).await
    }
}
