use super::*;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use serde_json::{json, Value};
use std::collections::HashMap;

pub(super) struct BatchRequest {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
}
impl GraphProvider {
    pub(super) async fn mutation_plan(
        &self,
        reference: &MessageRef,
        change: &Mutation,
    ) -> Result<(BatchRequest, Option<String>)> {
        let path = self.message_path(reference)?;
        let (method, path, body, destination) = match change.clone() {
            Mutation::Seen(value) => (
                Method::PATCH,
                path,
                serde_json::json!({"isRead":value}),
                None,
            ),
            Mutation::Starred(value) => (
                Method::PATCH,
                path,
                serde_json::json!({"flag":{"flagStatus":if value {"flagged"} else {"notFlagged"}}}),
                None,
            ),
            change => {
                let copy = matches!(change, Mutation::Copy(_));
                let name = match change {
                    Mutation::Archive => self
                        .mapping
                        .archive
                        .clone()
                        .unwrap_or_else(|| "Archive".into()),
                    Mutation::Trash => self.mapping.trash.clone().unwrap_or_else(|| "Trash".into()),
                    Mutation::Spam => "Junk".into(),
                    Mutation::Move(name) | Mutation::Copy(name) => name,
                    _ => unreachable!(),
                };
                let folder = self.folder(&name).await?;
                if folder.folder.name == reference.folder {
                    return Err(error(
                        ErrorKind::InvalidInput,
                        "source and destination identical",
                    ));
                }
                (
                    Method::POST,
                    format!("{path}/{}", if copy { "copy" } else { "move" }),
                    serde_json::json!({"destinationId":folder.id}),
                    Some(name),
                )
            }
        };

        Ok((
            BatchRequest {
                method,
                path,
                body: Some(body),
            },
            destination,
        ))
    }
    pub async fn mutate_batch(
        &self,
        changes: &[(MessageRef, Mutation)],
    ) -> Result<Vec<Result<MutationResult>>> {
        if changes.is_empty() || changes.len() > 20 {
            return Err(error(
                ErrorKind::InvalidInput,
                "invalid Graph mutation batch",
            ));
        }
        let mut plans = Vec::new();
        let mut positions = Vec::new();
        let mut results =
            vec![Err(error(ErrorKind::Uncertain, "Graph batch outcome unknown")); changes.len()];
        for (index, (reference, change)) in changes.iter().enumerate() {
            match self.mutation_plan(reference, change).await {
                Ok((plan, destination)) => {
                    plans.push(plan);
                    positions.push((index, destination));
                }
                Err(e) => results[index] = Err(e),
            }
        }
        if plans.is_empty() {
            return Ok(results);
        }
        let responses = self.batch(plans).await?;
        for ((index, destination), response) in positions.into_iter().zip(responses) {
            results[index] = response.and_then(|body| {
                let destination_reference = if let Some(folder) = &destination {
                    let id = body["id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            error(ErrorKind::Uncertain, "Graph mutation outcome unknown")
                        })?;
                    Some(self.reference(folder, id.into()))
                } else {
                    None
                };
                Ok(MutationResult {
                    destination,
                    destination_reference,
                    requires_rescan: true,
                })
            });
        }
        Ok(results)
    }
    pub(super) async fn batch(&self, requests: Vec<BatchRequest>) -> Result<Vec<Result<Value>>> {
        if requests.is_empty() || requests.len() > 20 {
            return Err(error(ErrorKind::InvalidInput, "invalid Graph batch size"));
        }
        let mut payload = Vec::new();
        let writes = requests.iter().any(|r| r.method != Method::GET);
        for (index, request) in requests.iter().enumerate() {
            let url = self.url(&request.path)?;
            if !url.path().starts_with(&format!("{}me/", self.base.path())) {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "invalid Graph batch resource",
                ));
            }
            let mut relative = format!(
                "/{}",
                url.path()
                    .strip_prefix(self.base.path())
                    .ok_or_else(invalid)?
            );
            if let Some(query) = url.query() {
                relative.push('?');
                relative.push_str(query);
            }
            let mut item = json!({"id":index.to_string(),"method":request.method.as_str(),"url":relative,"headers":{"Prefer":"IdType=\"ImmutableId\""}});
            if let Some(body) = &request.body {
                item["body"] = body.clone();
                item["headers"]["Content-Type"] = json!("application/json");
            }
            payload.push(item);
        }
        let failure = || {
            error(
                if writes {
                    ErrorKind::Uncertain
                } else {
                    ErrorKind::Retryable
                },
                "Graph batch outcome unavailable",
            )
        };
        let response = self
            .request(Method::POST, "$batch")
            .await?
            .json(&json!({"requests":payload}))
            .send()
            .await
            .map_err(|_| failure())?;
        if writes && response.status().is_server_error() {
            return Err(failure());
        }
        let response = self.checked(response).await?;
        let body: Value =
            Self::read_json(response)
                .await
                .map_err(|e| if writes { failure() } else { e })?;
        let items = body["responses"].as_array().ok_or_else(failure)?;
        let mut responses = HashMap::new();
        for item in items {
            let id = item["id"]
                .as_str()
                .ok_or_else(failure)?
                .parse::<usize>()
                .map_err(|_| failure())?;
            if id >= requests.len() || responses.insert(id, item).is_some() {
                return Err(failure());
            }
        }
        let mut results = Vec::with_capacity(requests.len());
        for (id, request) in requests.iter().enumerate() {
            let Some(item) = responses.get(&id) else {
                results.push(Err(failure()));
                continue;
            };
            let Some(status) = item["status"].as_u64().and_then(|s| u16::try_from(s).ok()) else {
                results.push(Err(failure()));
                continue;
            };
            if (200..300).contains(&status) {
                results.push(Ok(item["body"].clone()));
                continue;
            }
            let kind = match status {
                401 | 403 => ErrorKind::Authentication,
                404 | 410 => ErrorKind::StaleReference,
                429 | 503 => {
                    let seconds = item["headers"]
                        .as_object()
                        .and_then(|h| {
                            h.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
                        })
                        .and_then(|(_, v)| v.as_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(60)
                        .clamp(1, 86400);
                    *self.cooldown.lock().await =
                        Some(Instant::now() + Duration::from_secs(seconds));
                    ErrorKind::Retryable
                }
                500..=599 if request.method != Method::GET => ErrorKind::Uncertain,
                400..=499 => ErrorKind::InvalidInput,
                _ => ErrorKind::Retryable,
            };
            results.push(Err(error(kind, "Graph batch request rejected")));
        }
        Ok(results)
    }
    pub(super) async fn metadata_batch(
        &self,
        folder: &str,
        ids: &[String],
    ) -> Result<Vec<Result<Metadata>>> {
        let requests = ids
            .iter()
            .map(|id| BatchRequest {
                method: Method::GET,
                path: format!(
                    "me/messages/{}?$select=id,isRead,flag,receivedDateTime,internetMessageHeaders",
                    segment(id)
                ),
                body: None,
            })
            .collect();
        let results = self.batch(requests).await?;
        let mut metadata = Vec::new();
        for (id, result) in ids.iter().zip(results) {
            let result = match result {
                Ok(body) => {
                    let message: RemoteMessage =
                        serde_json::from_value(body).map_err(|_| invalid())?;
                    if &message.id != id {
                        return Err(invalid());
                    }
                    let mut row = self.metadata(folder, message)?;
                    self.complete_headers(&mut row).await?;
                    Ok(row)
                }
                Err(e) => Err(e),
            };
            metadata.push(result);
        }
        Ok(metadata)
    }
    pub(super) async fn complete_headers(&self, row: &mut Metadata) -> Result<()> {
        if row.headers == b"\r\n" {
            let raw = self.fetch(&row.reference, 25 * 1024 * 1024).await?;
            let end = raw
                .windows(4)
                .position(|v| v == b"\r\n\r\n")
                .ok_or_else(invalid)?
                + 4;
            row.size = raw.len() as u32;
            row.headers = raw[..end].to_vec();
        }
        Ok(())
    }
    pub(super) async fn fetch_mime_batch(
        &self,
        references: &[MessageRef],
        max_bytes: u32,
    ) -> Result<Vec<Vec<u8>>> {
        if references.is_empty() || references.len() > 50 {
            return Err(error(
                ErrorKind::InvalidInput,
                "invalid Graph body batch size",
            ));
        }
        let mut bodies = Vec::new();
        let mut total = 0usize;
        for chunk in references.chunks(20) {
            let requests = chunk
                .iter()
                .map(|r| {
                    Ok(BatchRequest {
                        method: Method::GET,
                        path: format!("{}/$value", self.message_path(r)?),
                        body: None,
                    })
                })
                .collect::<Result<_>>()?;
            for result in self.batch(requests).await? {
                let body = result?;
                let encoded = body.as_str().ok_or_else(invalid)?;
                if encoded.len() > (max_bytes as usize).saturating_mul(4) / 3 + 8 {
                    return Err(error(
                        ErrorKind::Unsupported,
                        "message exceeds content limit",
                    ));
                }
                let bytes = STANDARD
                    .decode(encoded)
                    .or_else(|_| URL_SAFE.decode(encoded))
                    .or_else(|_| URL_SAFE_NO_PAD.decode(encoded))
                    .map_err(|_| invalid())?;
                total = total.saturating_add(bytes.len());
                if bytes.len() > max_bytes as usize || total > 16 * 1024 * 1024 {
                    return Err(error(
                        ErrorKind::Unsupported,
                        "Graph body batch exceeds content limit",
                    ));
                }
                bodies.push(bytes);
            }
        }
        Ok(bodies)
    }
}
