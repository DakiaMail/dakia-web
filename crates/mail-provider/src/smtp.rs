//! The plan's desktop fallback: lettre's low-level connection, not its automatic
//! send/retry transport. Envelope rejection aborts before any DATA body is sent.
use crate::{mime::PreparedMessage, *};
use lettre::transport::smtp::{
    authentication::{Credentials as SmtpCredentials, Mechanism, DEFAULT_MECHANISMS},
    client::{AsyncSmtpConnection, TlsParameters},
    commands::{Data, Mail, Rcpt},
    extension::{ClientId, Extension, MailBodyParameter, MailParameter},
};
use std::future::Future;
use tokio::time::{timeout_at, Instant};

pub struct SmtpProvider {
    config: Config,
    credentials: Credentials,
    pub(crate) oauth: Option<std::sync::Arc<dyn AccessToken>>,
    tls: TlsParameters,
}
impl SmtpProvider {
    pub fn with_oauth(mut self, source: std::sync::Arc<dyn AccessToken>) -> Self {
        self.oauth = Some(source);
        self
    }
    pub fn new(config: Config, credentials: Credentials) -> Result<Self> {
        let tls = TlsParameters::new(config.smtp.host.clone())
            .map_err(|_| error(ErrorKind::InvalidInput, "invalid SMTP TLS configuration"))?;
        Ok(Self {
            config,
            credentials,
            oauth: None,
            tls,
        })
    }
    pub fn with_root(
        mut self,
        certificate: lettre::transport::smtp::client::Certificate,
    ) -> Result<Self> {
        self.tls = TlsParameters::builder(self.config.smtp.host.clone())
            .add_root_certificate(certificate)
            .build()
            .map_err(|_| error(ErrorKind::InvalidInput, "invalid SMTP trust root"))?;
        Ok(self)
    }
    async fn connect(&self, deadline: Instant) -> Result<AsyncSmtpConnection> {
        let id = ClientId::default();
        let mut c = before(
            deadline,
            AsyncSmtpConnection::connect_tokio1(
                (self.config.smtp.host.as_str(), self.config.smtp.port),
                None,
                &id,
                if self.config.smtp.security == Security::Tls {
                    Some(self.tls.clone())
                } else {
                    None
                },
                None,
            ),
        )
        .await?;
        if self.config.smtp.security == Security::StartTls {
            before(deadline, c.starttls(self.tls.clone(), &id)).await?;
        }
        let token = if let Some(source) = &self.oauth {
            Some(
                timeout_at(deadline, source.access_token())
                    .await
                    .map_err(|_| {
                        error(
                            ErrorKind::Retryable,
                            "SMTP deadline exceeded before message body",
                        )
                    })??,
            )
        } else {
            None
        };
        before(
            deadline,
            c.auth(
                if token.is_some() {
                    &[Mechanism::Xoauth2]
                } else {
                    DEFAULT_MECHANISMS
                },
                &SmtpCredentials::new(
                    self.credentials.username.clone(),
                    token.unwrap_or_else(|| self.credentials.password.clone()),
                ),
            ),
        )
        .await?;
        Ok(c)
    }
    /// QUIT after authentication. This never starts an envelope.
    pub async fn probe(&self) -> Result<()> {
        let deadline = Instant::now() + self.config.timeout;
        let mut c = self.connect(deadline).await?;
        before(deadline, c.quit()).await?;
        Ok(())
    }
    /// `begin_data` MUST durably transition claimed -> submitting before returning.
    /// The callback runs after ALL RCPT commands succeed, before DATA. If process
    /// death leaves submitting persisted, recovery must classify it unknown.
    /// No adapter retry occurs, and no filing operation occurs in this method.
    pub async fn submit<F, Fut>(
        &self,
        prepared: &PreparedMessage,
        begin_data: F,
    ) -> SubmissionOutcome
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        if prepared.account_id != self.config.account_id
            || prepared.primary_address != self.config.primary_address
        {
            return SubmissionOutcome::Failed {
                kind: ErrorKind::InvalidInput,
            };
        }
        let deadline = Instant::now() + self.config.timeout;
        let setup = async {
            let mut c = self.connect(deadline).await?;
            let envelope = &prepared.envelope;
            let mut options = Vec::new();
            if envelope
                .from()
                .into_iter()
                .chain(envelope.to())
                .any(|a| !a.to_string().is_ascii())
            {
                if !c.server_info().supports_feature(Extension::SmtpUtfEight) {
                    return Err(error(ErrorKind::Unsupported, "SMTPUTF8 not supported"));
                }
                options.push(MailParameter::SmtpUtfEight);
            }
            if !prepared.raw.is_ascii() {
                if !c.server_info().supports_feature(Extension::EightBitMime) {
                    return Err(error(ErrorKind::Unsupported, "8BITMIME not supported"));
                }
                options.push(MailParameter::Body(MailBodyParameter::EightBitMime));
            }
            before(
                deadline,
                c.command(Mail::new(envelope.from().cloned(), options)),
            )
            .await?;
            for recipient in envelope.to() {
                before(
                    deadline,
                    c.command(Rcpt::new(recipient.clone(), Vec::new())),
                )
                .await?;
            }
            // Rejecting any recipient drops the connection before DATA. Never send
            // a partial envelope to the remaining accepted recipients.
            tokio::time::timeout_at(deadline, begin_data())
                .await
                .map_err(|_| error(ErrorKind::Retryable, "submission transition timed out"))??;
            before(deadline, c.command(Data)).await?;
            Ok(c)
        }
        .await;
        let mut c = match setup {
            Ok(c) => c,
            Err(e) => return SubmissionOutcome::Failed { kind: e.kind },
        };
        match timeout_at(deadline, c.message(&prepared.raw)).await {
            Ok(Ok(response)) => SubmissionOutcome::Accepted {
                response: response.message().collect::<Vec<_>>().join(" "),
            },
            Ok(Err(e)) if e.is_permanent() || e.is_transient() => {
                SubmissionOutcome::Failed { kind: classify(&e) }
            }
            // This includes a failed/partial body write. Conservative uncertainty
            // is safe; treating a possibly accepted message as retryable is not.
            _ => SubmissionOutcome::DeliveryUnknown,
        }
    }
}
fn classify(e: &lettre::transport::smtp::Error) -> ErrorKind {
    if e.is_timeout() || e.is_transient() {
        ErrorKind::Retryable
    } else if e
        .status()
        .is_some_and(|s| s.to_string().starts_with("535") || s.to_string().starts_with("534"))
    {
        ErrorKind::Authentication
    } else {
        ErrorKind::Unsupported
    }
}
async fn before<T>(
    deadline: Instant,
    f: impl Future<Output = std::result::Result<T, lettre::transport::smtp::Error>>,
) -> Result<T> {
    match timeout_at(deadline, f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(error(
            classify(&e),
            if classify(&e) == ErrorKind::Authentication {
                "SMTP authentication rejected before message body"
            } else if e.is_transient() {
                "SMTP temporary rejection before message body"
            } else if e.status().is_none() {
                "SMTP transport interrupted before message body"
            } else {
                "SMTP command failed before message body"
            },
        )),
        Err(_) => Err(error(
            ErrorKind::Retryable,
            "SMTP deadline exceeded before message body",
        )),
    }
}
