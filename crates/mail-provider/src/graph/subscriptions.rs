use super::*;
use serde_json::json;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSubscription {
    pub id: String,
    pub expiration_date_time: String,
}
impl GraphProvider {
    pub async fn subscription(
        &self,
        notification_url: &str,
        client_state: &str,
        existing: Option<&str>,
    ) -> Result<GraphSubscription> {
        let expiration = (chrono::Utc::now() + chrono::Duration::hours(70))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let (method, path, body) = if let Some(id) = existing {
            (
                Method::PATCH,
                format!("subscriptions/{}", segment(id)),
                json!({"expirationDateTime":expiration}),
            )
        } else {
            (
                Method::POST,
                "subscriptions".into(),
                json!({
                    "changeType":"created,updated,deleted","notificationUrl":notification_url,
                    "lifecycleNotificationUrl":notification_url,"resource":"me/messages",
                    "expirationDateTime":expiration,"clientState":client_state
                }),
            )
        };
        let response = self
            .request(method, &path)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|_| error(ErrorKind::Retryable, "Graph subscription unavailable"))?;
        let mut subscription: GraphSubscription =
            Self::read_json(self.checked(response).await?).await?;
        let expires = chrono::DateTime::parse_from_rfc3339(&subscription.expiration_date_time)
            .map_err(|_| invalid())?;
        if subscription.id.is_empty()
            || expires <= chrono::Utc::now()
            || expires > chrono::Utc::now() + chrono::Duration::hours(72)
        {
            return Err(invalid());
        }
        subscription.expiration_date_time = expires
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        Ok(subscription)
    }
    pub async fn prepare_unsubscribe(&self, id: &str) -> Result<reqwest::RequestBuilder> {
        self.request(Method::DELETE, &format!("subscriptions/{}", segment(id)))
            .await
    }
    pub async fn unsubscribe(&self, id: &str) -> Result<()> {
        let response = self
            .prepare_unsubscribe(id)
            .await?
            .send()
            .await
            .map_err(|_| error(ErrorKind::Retryable, "Graph subscription unavailable"))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        self.checked(response).await?;
        Ok(())
    }
}
