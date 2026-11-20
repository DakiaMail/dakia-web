//! Connection credentials never cross the repository boundary in plaintext.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentConnection {
    pub id: String,
    #[serde(skip_serializing)]
    pub user_id: String,
    pub name: String,
    pub allow_send: bool,
    pub allow_delete: bool,
    pub revoked: bool,
    pub created_at: i64,
}
macro_rules! connection {
    ($r:ident) => { AgentConnection { id:$r.get("id"), user_id:$r.get("user_id"), name:$r.get("name"), allow_send:$r.get::<i32,_>("allow_send")!=0, allow_delete:$r.get::<i32,_>("allow_delete")!=0, revoked:$r.get::<i32,_>("revoked")!=0, created_at:$r.get("created_at") } };
}
mod oauth;
pub use oauth::{AgentOAuthCode,AgentOAuthToken};
impl ControlRepository {
    /// None means the caller owns a new request. Existing requests return their
    /// outcome, or delivery_unknown after a concurrent/interrupted attempt.
    pub async fn claim_agent_deletion(&self,c:&str,request:&str,payload:&str)->Result<Option<String>> {
        let changed=execute!(self,"INSERT INTO agent_destructive_requests(connection_id,request_hash,payload_hash) VALUES(?,?,?) ON CONFLICT(connection_id,request_hash) DO NOTHING","INSERT INTO agent_destructive_requests(connection_id,request_hash,payload_hash) VALUES($1,$2,$3) ON CONFLICT(connection_id,request_hash) DO NOTHING",[c,request,payload]);
        if changed==1 {return Ok(None)}
        let (old,result)=optional!(self,"SELECT payload_hash,result FROM agent_destructive_requests WHERE connection_id=? AND request_hash=?","SELECT payload_hash,result FROM agent_destructive_requests WHERE connection_id=$1 AND request_hash=$2",[c,request],r=>(r.get::<String,_>("payload_hash"),r.get::<Option<String>,_>("result"))).ok_or(crate::Error::NotFound)?;
        if old!=payload {return Err(crate::Error::Conflict)}
        Ok(Some(result.unwrap_or_else(||"{\"outcome\":\"unknown\"}".into())))
    }
    pub async fn finish_agent_deletion(&self,c:&str,request:&str,result:&str)->Result<()> {
        execute!(self,"UPDATE agent_destructive_requests SET result=? WHERE connection_id=? AND request_hash=? AND result IS NULL","UPDATE agent_destructive_requests SET result=$1 WHERE connection_id=$2 AND request_hash=$3 AND result IS NULL",[result,c,request]);Ok(())
    }
    pub async fn bind_agent_send(&self, connection: &str, request: &str, payload: &str, account: &str, operation: &str) -> Result<()> {
        execute!(self,"INSERT INTO agent_send_requests(connection_id,request_hash,payload_hash,account_id,operation_id) VALUES(?,?,?,?,?) ON CONFLICT(connection_id,request_hash) DO NOTHING","INSERT INTO agent_send_requests(connection_id,request_hash,payload_hash,account_id,operation_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT(connection_id,request_hash) DO NOTHING",[connection,request,payload,account,operation]);
        let previous=optional!(self,"SELECT payload_hash FROM agent_send_requests WHERE connection_id=? AND request_hash=?","SELECT payload_hash FROM agent_send_requests WHERE connection_id=$1 AND request_hash=$2",[connection,request],r=>r.get::<String,_>("payload_hash"));
        if previous.as_deref()!=Some(payload) {return Err(crate::Error::Conflict)}
        Ok(())
    }
    pub async fn agent_send(&self, connection: &str, request: &str) -> Result<(String,String)> {
        optional!(self,"SELECT account_id,operation_id FROM agent_send_requests WHERE connection_id=? AND request_hash=?","SELECT account_id,operation_id FROM agent_send_requests WHERE connection_id=$1 AND request_hash=$2",[connection,request],r=>(r.get("account_id"),r.get("operation_id"))).ok_or(crate::Error::NotFound)
    }
    pub async fn create_agent(&self, user: &str, id: &str, name: &str, token_hash: &str, at: i64) -> Result<AgentConnection> {
        execute!(self,"INSERT INTO agent_connections(id,user_id,name,token_hash,created_at) VALUES(?,?,?,?,?)","INSERT INTO agent_connections(id,user_id,name,token_hash,created_at) VALUES($1,$2,$3,$4,$5)",[id,user,name,token_hash,at]);
        self.agent(user,id).await
    }
    pub async fn agent(&self, user: &str, id: &str) -> Result<AgentConnection> {
        optional!(self,"SELECT * FROM agent_connections WHERE user_id=? AND id=?","SELECT * FROM agent_connections WHERE user_id=$1 AND id=$2",[user,id],r=>connection!(r)).ok_or(crate::Error::NotFound)
    }
    pub async fn authenticate_agent(&self, token_hash: &str) -> Result<Option<AgentConnection>> {
        Ok(optional!(self,"SELECT * FROM agent_connections WHERE token_hash=? AND revoked=0","SELECT * FROM agent_connections WHERE token_hash=$1 AND revoked=0",[token_hash],r=>connection!(r)))
    }
    pub async fn agents(&self, user: &str) -> Result<Vec<AgentConnection>> {
        Ok(match &self.backend {
            Backend::Sqlite(db)=>sqlx::query("SELECT * FROM agent_connections WHERE user_id=? AND revoked=0 ORDER BY created_at,id").bind(user).fetch_all(db).await?.into_iter().map(|r|connection!(r)).collect(),
            Backend::Postgres(db)=>sqlx::query("SELECT * FROM agent_connections WHERE user_id=$1 AND revoked=0 ORDER BY created_at,id").bind(user).fetch_all(db).await?.into_iter().map(|r|connection!(r)).collect(),
        })
    }
    pub async fn edit_agent(&self, user: &str, id: &str, name: &str, send: bool, delete: bool) -> Result<AgentConnection> {
        let changed=execute!(self,"UPDATE agent_connections SET name=?,allow_send=?,allow_delete=? WHERE user_id=? AND id=? AND revoked=0","UPDATE agent_connections SET name=$1,allow_send=$2,allow_delete=$3 WHERE user_id=$4 AND id=$5 AND revoked=0",[name,i32::from(send),i32::from(delete),user,id]);
        if changed==0 {return Err(crate::Error::NotFound)}
        self.agent(user,id).await
    }
    pub async fn revoke_agent(&self, user: &str, id: &str) -> Result<()> {
        let changed=execute!(self,"UPDATE agent_connections SET revoked=1 WHERE user_id=? AND id=?","UPDATE agent_connections SET revoked=1 WHERE user_id=$1 AND id=$2",[user,id]);
        if changed==0 {return Err(crate::Error::NotFound)}
        Ok(())
    }
    pub async fn audit_agent(&self, c: &AgentConnection, action: &str, outcome: &str, account: Option<&str>, resource: Option<&str>, request: Option<&str>) -> Result<()> {
        let at=chrono::Utc::now().timestamp();
        let id=uuid::Uuid::new_v4().to_string();
        execute!(self,"DELETE FROM agent_audit WHERE at<?","DELETE FROM agent_audit WHERE at<$1",[at-90*86400]);
        execute!(self,"INSERT INTO agent_audit(id,at,user_id,connection_id,action,outcome,account_id,resource_id,request_id) VALUES(?,?,?,?,?,?,?,?,?)","INSERT INTO agent_audit(id,at,user_id,connection_id,action,outcome,account_id,resource_id,request_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",[&id,at,&c.user_id,&c.id,action,outcome,account,resource,request]);
        Ok(())
    }
}
