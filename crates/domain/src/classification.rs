use super::*;

values!(InboxKind {
    Personal,
    Automated
});
values!(DetectedType {
    Receipt,
    Invoice,
    Order,
    Shipping,
    EventInvitation,
    VerificationCode,
    FlightBooking,
    Newsletter,
    Offer,
    SecurityAlert
});
values!(ClassificationSource {
    Automatic,
    Message,
    Sender
});
values!(DetailAction { Copy, Open });
dto!(MailDetail { email_type: DetectedType, label: String, value: String, action: Option<DetailAction> });
dto!(AttentionReason { label: String, expires_at: Option<String> });
dto!(Classification {
    kind: InboxKind, status: CategoryStatus, types: Vec<DetectedType>, details: Vec<MailDetail>,
    attention: Vec<AttentionReason>, attention_override: Option<bool>, source: ClassificationSource,
    policy_revision: String
});
impl Default for Classification {
    fn default() -> Self {
        Self {
            kind: InboxKind::Personal,
            status: CategoryStatus::Pending,
            types: vec![],
            details: vec![],
            attention: vec![],
            attention_override: None,
            source: ClassificationSource::Automatic,
            policy_revision: String::new(),
        }
    }
}
impl Classification {
    pub fn needs_attention(&self, now: &str) -> bool {
        self.attention_override.unwrap_or_else(|| {
            self.attention
                .iter()
                .any(|a| a.expires_at.as_deref().is_none_or(|expiry| expiry > now))
        })
    }
    pub fn legacy_category(&self) -> &'static str {
        if self.kind == InboxKind::Personal {
            "people"
        } else if self
            .types
            .iter()
            .any(|t| matches!(t, DetectedType::Newsletter | DetectedType::Offer))
        {
            "newsletters"
        } else if self.types.iter().any(|t| {
            matches!(
                t,
                DetectedType::Receipt
                    | DetectedType::Invoice
                    | DetectedType::Order
                    | DetectedType::Shipping
                    | DetectedType::FlightBooking
            )
        }) {
            "transactions"
        } else {
            "notifications"
        }
    }
}
impl Message {
    pub fn inbox_kind(&self) -> InboxKind {
        self.classification
            .as_ref()
            .map(|c| c.kind.clone())
            .unwrap_or(InboxKind::Personal)
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClassificationFilter {
    pub inbox_kind: Option<InboxKind>,
    pub email_type: Option<DetectedType>,
    pub attention: bool,
}
impl ClassificationFilter {
    pub fn matches(&self, message: &Message, now: &str) -> bool {
        self.inbox_kind
            .as_ref()
            .is_none_or(|kind| *kind == message.inbox_kind())
            && self.email_type.as_ref().is_none_or(|t| {
                message
                    .classification
                    .as_ref()
                    .is_some_and(|c| c.types.contains(t))
            })
            && (!self.attention
                || message
                    .classification
                    .as_ref()
                    .is_some_and(|c| c.needs_attention(now)))
    }
}
