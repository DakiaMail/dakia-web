use super::*;
impl GraphProvider {
    async fn retry_delay(&self) -> u64 {
        self.cooldown
            .lock()
            .await
            .map(|until| {
                until
                    .saturating_duration_since(Instant::now())
                    .as_secs()
                    .max(1)
            })
            .unwrap_or(30)
    }
    pub async fn has_sent_message(&self, message_id: &str) -> Result<bool> {
        if message_id.len() > 998 || message_id.contains(['\r', '\n']) {
            return Err(error(ErrorKind::InvalidInput, "invalid sent message id"));
        }
        let literal = format!(
            "<{}>",
            message_id
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
        )
        .replace('\'', "''");
        let mut url = self.url("me/mailFolders/sentitems/messages")?;
        url.query_pairs_mut().extend_pairs([
            (
                "$filter",
                format!("internetMessageId eq '{literal}'").as_str(),
            ),
            ("$select", "id"),
            ("$top", "1"),
        ]);
        let page: Collection<serde_json::Value> = self.json(url.as_str()).await?;
        if page
            .value
            .iter()
            .any(|m| m["id"].as_str().is_none_or(str::is_empty))
        {
            return Err(invalid());
        }
        Ok(!page.value.is_empty())
    }
    pub(super) async fn submit_mime(
        &self,
        prepared: &PreparedMessage,
        gate: &dyn SubmissionGate,
    ) -> SubmissionOutcome {
        if prepared.account_id != self.account {
            return SubmissionOutcome::Failed {
                kind: ErrorKind::InvalidInput,
            };
        }
        // shortcut: larger messages need Graph draft attachment upload sessions.
        if prepared.raw().len() > 4 * 1024 * 1024 {
            return SubmissionOutcome::Failed {
                kind: ErrorKind::Unsupported,
            };
        }
        let mut raw = Vec::new();
        let parsed = match mail_parser::MessageParser::default().parse(prepared.raw()) {
            Some(parsed) => parsed,
            None => {
                return SubmissionOutcome::Failed {
                    kind: ErrorKind::InvalidInput,
                }
            }
        };
        let visible: Vec<_> = parsed
            .to()
            .into_iter()
            .chain(parsed.cc())
            .flat_map(|a| a.iter())
            .filter_map(|a| a.address())
            .collect();
        let hidden: Vec<_> = prepared
            .recipients()
            .into_iter()
            .filter(|a| !visible.iter().any(|v| v.eq_ignore_ascii_case(a)))
            .collect();
        if !hidden.is_empty() {
            raw.extend_from_slice(format!("Bcc: {}\r\n", hidden.join(", ")).as_bytes());
        }
        raw.extend_from_slice(prepared.raw());
        if raw.len() > 4 * 1024 * 1024 {
            return SubmissionOutcome::Failed {
                kind: ErrorKind::Unsupported,
            };
        }
        let request = match self.request(Method::POST, "me/sendMail").await {
            Ok(request) => request
                .header("Content-Type", "text/plain")
                .body(STANDARD.encode(raw)),
            Err(e) => {
                return if e.kind == ErrorKind::Retryable {
                    SubmissionOutcome::Deferred {
                        retry_after_secs: self.retry_delay().await,
                    }
                } else {
                    SubmissionOutcome::Failed { kind: e.kind }
                }
            }
        };
        if let Err(e) = gate.begin_data().await {
            return SubmissionOutcome::Failed { kind: e.kind };
        }
        match request.send().await {
            Ok(response) if response.status() == StatusCode::ACCEPTED => {
                SubmissionOutcome::Accepted {
                    response: "Graph accepted".into(),
                }
            }
            Ok(response) if matches!(response.status().as_u16(), 502 | 504) => {
                SubmissionOutcome::DeliveryUnknown
            }
            Ok(response)
                if response.status() == StatusCode::TOO_MANY_REQUESTS
                    || response.status().is_server_error() =>
            {
                let delay = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(60)
                    .clamp(1, 86400);
                *self.cooldown.lock().await = Some(Instant::now() + Duration::from_secs(delay));
                SubmissionOutcome::Deferred {
                    retry_after_secs: delay,
                }
            }
            Ok(response) if !response.status().is_success() => {
                let kind = if response.status() == StatusCode::UNAUTHORIZED {
                    ErrorKind::Authentication
                } else {
                    ErrorKind::InvalidInput
                };
                SubmissionOutcome::Failed { kind }
            }
            Err(error) if error.is_connect() => SubmissionOutcome::Deferred {
                retry_after_secs: 30,
            },
            _ => SubmissionOutcome::DeliveryUnknown,
        }
    }
}
