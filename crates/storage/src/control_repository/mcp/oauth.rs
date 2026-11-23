use super::*;
#[derive(Clone)]
pub struct AgentOAuthCode {
    pub hash: String,
    pub connection: String,
    pub client: String,
    pub redirect: String,
    pub challenge: String,
    pub resource: String,
    pub expires: i64,
}
pub struct AgentOAuthToken {
    pub access_hash: String,
    pub refresh_hash: String,
    pub connection: String,
    pub client: String,
    pub resource: String,
    pub expires: i64,
    pub refresh_expires: i64,
}
impl ControlRepository {
    pub async fn register_agent_client(
        &self,
        id: &str,
        document: &str,
        expires: i64,
    ) -> Result<()> {
        let at = chrono::Utc::now().timestamp();
        execute!(
            self,
            "DELETE FROM agent_oauth_clients WHERE expires_at<?",
            "DELETE FROM agent_oauth_clients WHERE expires_at<$1",
            [at]
        );
        execute!(
            self,
            "INSERT INTO agent_oauth_clients(id,document,expires_at) VALUES(?,?,?)",
            "INSERT INTO agent_oauth_clients(id,document,expires_at) VALUES($1,$2,$3)",
            [id, document, expires]
        );
        Ok(())
    }
    pub async fn agent_client(&self, id: &str, at: i64) -> Result<Option<String>> {
        Ok(
            optional!(self,"SELECT document FROM agent_oauth_clients WHERE id=? AND expires_at>?","SELECT document FROM agent_oauth_clients WHERE id=$1 AND expires_at>$2",[id,at],r=>r.get("document")),
        )
    }
    pub async fn create_agent_code(
        &self,
        c: &AgentOAuthCode,
        new_connection: Option<(&str, &str, &str)>,
    ) -> Result<()> {
        let at = chrono::Utc::now().timestamp();
        transaction!(self, tx, sqlite, {
            if let Some((user, name, unused_token_hash)) = new_connection {
                sqlx::query(statement!(
                    sqlite,
                    "INSERT INTO agent_connections(id,user_id,name,token_hash,created_at) VALUES(?,?,?,?,?)",
                    "INSERT INTO agent_connections(id,user_id,name,token_hash,created_at) VALUES($1,$2,$3,$4,$5)"
                ))
                .bind(&c.connection)
                .bind(user)
                .bind(name)
                .bind(unused_token_hash)
                .bind(at)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(statement!(
                sqlite,
                "DELETE FROM agent_oauth_codes WHERE expires_at<?",
                "DELETE FROM agent_oauth_codes WHERE expires_at<$1"
            ))
            .bind(at)
            .execute(&mut *tx)
            .await?;
            sqlx::query(statement!(
                sqlite,
                "INSERT INTO agent_oauth_codes(code_hash,connection_id,client_id,redirect_uri,challenge,resource,expires_at) VALUES(?,?,?,?,?,?,?)",
                "INSERT INTO agent_oauth_codes(code_hash,connection_id,client_id,redirect_uri,challenge,resource,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)"
            ))
            .bind(&c.hash)
            .bind(&c.connection)
            .bind(&c.client)
            .bind(&c.redirect)
            .bind(&c.challenge)
            .bind(&c.resource)
            .bind(c.expires)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        })
    }
    pub async fn consume_agent_code(&self, c: &AgentOAuthCode, at: i64) -> Result<Option<String>> {
        Ok(
            optional!(self,"DELETE FROM agent_oauth_codes WHERE code_hash=? AND client_id=? AND redirect_uri=? AND challenge=? AND resource=? AND expires_at>? AND EXISTS(SELECT 1 FROM agent_connections c WHERE c.id=agent_oauth_codes.connection_id AND c.revoked=0) RETURNING connection_id","DELETE FROM agent_oauth_codes WHERE code_hash=$1 AND client_id=$2 AND redirect_uri=$3 AND challenge=$4 AND resource=$5 AND expires_at>$6 AND EXISTS(SELECT 1 FROM agent_connections c WHERE c.id=agent_oauth_codes.connection_id AND c.revoked=0) RETURNING connection_id",[&c.hash,&c.client,&c.redirect,&c.challenge,&c.resource,at],r=>r.get("connection_id")),
        )
    }
    pub async fn create_agent_token(&self, t: &AgentOAuthToken) -> Result<()> {
        execute!(self,"INSERT INTO agent_oauth_tokens(access_hash,refresh_hash,connection_id,client_id,resource,expires_at,refresh_expires_at) VALUES(?,?,?,?,?,?,?)","INSERT INTO agent_oauth_tokens(access_hash,refresh_hash,connection_id,client_id,resource,expires_at,refresh_expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)",[&t.access_hash,&t.refresh_hash,&t.connection,&t.client,&t.resource,t.expires,t.refresh_expires]);
        Ok(())
    }
    pub async fn rotate_agent_token(
        &self,
        previous: &str,
        t: &AgentOAuthToken,
        at: i64,
    ) -> Result<bool> {
        let changed=execute!(self,"UPDATE agent_oauth_tokens SET access_hash=?,refresh_hash=?,expires_at=? WHERE refresh_hash=? AND client_id=? AND resource=? AND refresh_expires_at>? AND EXISTS(SELECT 1 FROM agent_connections c WHERE c.id=agent_oauth_tokens.connection_id AND c.revoked=0)","UPDATE agent_oauth_tokens SET access_hash=$1,refresh_hash=$2,expires_at=$3 WHERE refresh_hash=$4 AND client_id=$5 AND resource=$6 AND refresh_expires_at>$7 AND EXISTS(SELECT 1 FROM agent_connections c WHERE c.id=agent_oauth_tokens.connection_id AND c.revoked=0)",[&t.access_hash,&t.refresh_hash,t.expires,previous,&t.client,&t.resource,at]);
        Ok(changed == 1)
    }
    pub async fn authenticate_agent_oauth(
        &self,
        access: &str,
        resource: &str,
        at: i64,
    ) -> Result<Option<AgentConnection>> {
        Ok(
            optional!(self,"SELECT c.* FROM agent_connections c JOIN agent_oauth_tokens t ON t.connection_id=c.id WHERE t.access_hash=? AND t.resource=? AND t.expires_at>? AND c.revoked=0","SELECT c.* FROM agent_connections c JOIN agent_oauth_tokens t ON t.connection_id=c.id WHERE t.access_hash=$1 AND t.resource=$2 AND t.expires_at>$3 AND c.revoked=0",[access,resource,at],r=>connection!(r)),
        )
    }
}
