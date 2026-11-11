use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphCursor {
    pub folder_id: String,
    pub url: String,
    pub initial: bool,
    pub next: bool,
}
pub struct GraphDeltaPage {
    pub messages: Vec<Metadata>,
    pub updates: Vec<MutableMetadata>,
    pub removed: Vec<String>,
    pub cursor: GraphCursor,
}
pub struct GraphArrivalPage {
    pub messages: Vec<Metadata>,
    pub next: Option<String>,
}
impl GraphProvider {
    fn continuation(&self, cursor: &GraphCursor, folder_id: &str, suffix: &str) -> Result<String> {
        if cursor.folder_id != folder_id {
            return Err(error(
                ErrorKind::StaleReference,
                "Graph folder binding changed",
            ));
        }
        let url = self.url(&cursor.url)?;
        if !self.folder_resource(url.path(), folder_id, suffix) {
            return Err(error(ErrorKind::StaleReference, "Graph traversal changed"));
        }
        Ok(url.to_string())
    }
    pub async fn delta(
        &self,
        folder: &str,
        cursor: Option<&GraphCursor>,
        known: &dyn MetadataLookup,
    ) -> Result<GraphDeltaPage> {
        let remote = self.folder(folder).await?;
        let path = match cursor {
            Some(cursor) => self.continuation(cursor,&remote.id,"messages/delta")?,
            None => format!("me/mailFolders/{}/messages/delta?$select=id,internetMessageId,parentFolderId,isRead,flag&$orderby=receivedDateTime%20desc&$top=200",segment(&remote.id)),
        };
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Prefer",
            "IdType=\"ImmutableId\", odata.maxpagesize=200"
                .parse()
                .unwrap(),
        );
        let response = self
            .request(Method::GET, &path)
            .await?
            .headers(headers)
            .send()
            .await
            .map_err(|_| error(ErrorKind::Retryable, "Microsoft Graph unavailable"))?;
        let response = self.checked(response).await?;
        let body: Value = Self::read_json(response).await?;
        let items = body["value"].as_array().ok_or_else(invalid)?;
        if items.len() > 200 {
            return Err(invalid());
        }
        let next = body["@odata.nextLink"].as_str().filter(|s| !s.is_empty());
        let delta = body["@odata.deltaLink"].as_str().filter(|s| !s.is_empty());
        if next.is_some() == delta.is_some() {
            return Err(invalid());
        }
        let url = next.or(delta).unwrap().to_owned();
        let final_cursor = GraphCursor {
            folder_id: remote.id.clone(),
            url,
            initial: cursor.is_none_or(|c| c.initial) && next.is_some(),
            next: next.is_some(),
        };
        self.continuation(&final_cursor, &remote.id, "messages/delta")?;
        // A delta can repeat an id; only its last observation on this page wins.
        let mut latest = HashMap::new();
        for (index, item) in items.iter().enumerate() {
            let id = item["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(invalid)?;
            latest.insert(id, (index, item));
        }
        let mut items: Vec<_> = latest.into_values().collect();
        items.sort_by_key(|(index, _)| *index);
        let mut removed = Vec::new();
        let mut updates = Vec::new();
        let mut fetch = Vec::new();
        for (_, item) in items {
            let id = item["id"].as_str().unwrap().to_owned();
            if item.get("@removed").is_some() {
                removed.push(id);
                continue;
            }
            let identity = Identity::Graph {
                account_id: self.account.clone(),
                message_id: id.clone(),
            };
            let state = item["isRead"]
                .as_bool()
                .zip(item["flag"]["flagStatus"].as_str());
            if let Some((seen, flag)) =
                state.filter(|(_, flag)| matches!(*flag, "flagged" | "notFlagged" | "complete"))
            {
                if known.identity_known(&identity).await? {
                    updates.push(MutableMetadata {
                        reference: self.reference(folder, id),
                        identity,
                        labels: vec![],
                        seen,
                        starred: flag == "flagged",
                    });
                    continue;
                }
            }
            fetch.push(id);
        }
        let mut messages = Vec::new();
        for chunk in fetch.chunks(20) {
            for (id, result) in chunk.iter().zip(self.metadata_batch(folder, chunk).await?) {
                match result {
                    Ok(metadata) => messages.push(metadata),
                    Err(e) if e.kind == ErrorKind::StaleReference => removed.push(id.clone()),
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(GraphDeltaPage {
            messages,
            updates,
            removed,
            cursor: final_cursor,
        })
    }
    pub async fn arrivals(
        &self,
        folder: &str,
        since: &str,
        until: &str,
        next: Option<&str>,
    ) -> Result<GraphArrivalPage> {
        let remote = self.folder(folder).await?;
        let path = if let Some(next) = next {
            self.continuation(
                &GraphCursor {
                    folder_id: remote.id.clone(),
                    url: next.into(),
                    initial: false,
                    next: true,
                },
                &remote.id,
                "messages",
            )?
        } else {
            let since = chrono::DateTime::parse_from_rfc3339(since)
                .map_err(|_| invalid())?
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let until = chrono::DateTime::parse_from_rfc3339(until)
                .map_err(|_| invalid())?
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let mut url = self.url(&format!("me/mailFolders/{}/messages", segment(&remote.id)))?;
            url.query_pairs_mut().extend_pairs([
                ("$top", "50"),
                (
                    "$select",
                    "id,isRead,flag,receivedDateTime,internetMessageHeaders",
                ),
                (
                    "$filter",
                    &format!("receivedDateTime ge {since} and receivedDateTime le {until}"),
                ),
                ("$orderby", "receivedDateTime desc"),
            ]);
            url.into()
        };
        let page: Collection<RemoteMessage> = self.json(&path).await?;
        if page.value.len() > 50 {
            return Err(invalid());
        }
        if let Some(next) = &page.next {
            self.continuation(
                &GraphCursor {
                    folder_id: remote.id.clone(),
                    url: next.clone(),
                    initial: false,
                    next: true,
                },
                &remote.id,
                "messages",
            )?;
        }
        let mut messages = Vec::new();
        for message in page.value {
            let mut row = self.metadata(folder, message)?;
            self.complete_headers(&mut row).await?;
            messages.push(row);
        }
        Ok(GraphArrivalPage {
            messages,
            next: page.next,
        })
    }
}
