use serde_json::Value;
use uuid::Uuid;

/// One audit_log row. Written with the same transaction as the change it
/// records. audit_log is append-only (DB trigger) — no update or delete path.
pub struct AuditEntry<'a> {
    pub actor_id: Option<Uuid>,
    pub actor_role: Option<&'a str>,
    pub action: &'a str,
    pub entity_type: &'a str,
    pub entity_id: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

pub async fn write_audit(
    conn: &mut sqlx::PgConnection,
    entry: AuditEntry<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into audit_log (actor_id, actor_role, action, entity_type, entity_id, before, after)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(entry.actor_id)
    .bind(entry.actor_role)
    .bind(entry.action)
    .bind(entry.entity_type)
    .bind(entry.entity_id)
    .bind(entry.before)
    .bind(entry.after)
    .execute(conn)
    .await?;
    Ok(())
}
