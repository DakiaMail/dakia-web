//! Shared interpretation of mailbox search. Providers and local retrieval use
//! the same strict filters; quoted free text remains an explicit constraint.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextUnit {
    pub text: String,
    pub phrase: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailboxQuery {
    pub text: Vec<TextUnit>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub folder: Option<String>,
    pub since: Option<String>,
    pub before: Option<String>,
    pub seen: Option<bool>,
    pub starred: Option<bool>,
    #[serde(default)]
    pub has_attachment: bool,
    pub snoozed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidMailboxQuery;
impl std::fmt::Display for InvalidMailboxQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid mailbox search")
    }
}
impl std::error::Error for InvalidMailboxQuery {}
impl MailboxQuery {
    pub fn parse(input: &str) -> Result<Self, InvalidMailboxQuery> {
        if input.len() > 4096 || input.contains('\0') {
            return Err(InvalidMailboxQuery);
        }
        let mut tokens = vec![];
        let (mut word, mut quoted, mut escaped, mut phrase, mut literal) =
            (String::new(), false, false, false, false);
        for c in input.chars() {
            if escaped {
                word.push(c);
                escaped = false;
            } else if c == '\\' && quoted {
                escaped = true;
            } else if c == '"' {
                if word.is_empty() && !phrase {
                    literal = true;
                }
                quoted = !quoted;
                phrase = true;
            } else if c.is_whitespace() && !quoted {
                if !word.is_empty() {
                    tokens.push((std::mem::take(&mut word), phrase, literal));
                }
                phrase = false;
                literal = false;
            } else {
                word.push(c);
            }
        }
        if quoted || escaped {
            return Err(InvalidMailboxQuery);
        }
        if !word.is_empty() {
            tokens.push((word, phrase, literal));
        }
        let mut result = Self::default();
        for (text, phrase, literal) in tokens {
            let filter = if literal { None } else { text.split_once(':') };
            match filter {
                Some(("from", v)) => result.from = Some(required(v)?),
                Some(("to", v)) => result.to = Some(required(v)?),
                Some(("subject", v)) => result.subject = Some(required(v)?),
                Some(("folder", v)) => result.folder = Some(required(v)?),
                Some(("since" | "after", v)) => result.since = Some(date(v)?),
                Some(("before", v)) => result.before = Some(date(v)?),
                Some(("is", "read")) => result.seen = Some(true),
                Some(("is", "unread")) => result.seen = Some(false),
                Some(("is", "starred")) => result.starred = Some(true),
                Some(("has", "attachment")) => result.has_attachment = true,
                Some(("is", "snoozed")) => result.snoozed = true,
                _ => result.text.push(TextUnit { text, phrase }),
            }
        }
        Ok(result)
    }
    pub fn date_bounds(&self) -> Result<(Option<String>, Option<String>), InvalidMailboxQuery> {
        let iso = |day: &str| {
            chrono::NaiveDate::parse_from_str(day, "%d-%b-%Y")
                .map(|day| day.format("%Y-%m-%d").to_string())
                .map_err(|_| InvalidMailboxQuery)
        };
        Ok((
            self.since.as_deref().map(iso).transpose()?,
            self.before.as_deref().map(iso).transpose()?,
        ))
    }
    pub fn text_units(&self) -> Vec<String> {
        self.text.iter().map(|u| u.text.clone()).collect()
    }
    pub fn phrases(&self) -> Vec<String> {
        self.text
            .iter()
            .filter(|u| u.phrase)
            .map(|u| u.text.clone())
            .collect()
    }
    pub fn semantic_text(&self) -> String {
        self.text
            .iter()
            .map(|u| u.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
fn required(value: &str) -> Result<String, InvalidMailboxQuery> {
    if value.is_empty() {
        Err(InvalidMailboxQuery)
    } else {
        Ok(value.into())
    }
}
fn date(value: &str) -> Result<String, InvalidMailboxQuery> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .or_else(|_| chrono::NaiveDate::parse_from_str(value, "%d-%b-%Y"))
        .map(|d| d.format("%d-%b-%Y").to_string())
        .map_err(|_| InvalidMailboxQuery)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phrases_filters_and_unknown_operators_have_one_interpretation() {
        let q=MailboxQuery::parse(r#"from:"Maya Friend" folder:INBOX is:unread is:starred is:snoozed after:2026-01-02 before:03-Feb-2026 shipping "late delivery" "from:literal" future:syntax"#).unwrap();
        assert_eq!(q.from.as_deref(), Some("Maya Friend"));
        assert_eq!(q.folder.as_deref(), Some("INBOX"));
        assert_eq!(
            (q.seen, q.starred, q.snoozed),
            (Some(false), Some(true), true)
        );
        assert_eq!(q.since.as_deref(), Some("02-Jan-2026"));
        assert_eq!(q.before.as_deref(), Some("03-Feb-2026"));
        assert_eq!(
            q.date_bounds().unwrap(),
            (Some("2026-01-02".into()), Some("2026-02-03".into()))
        );
        assert_eq!(
            q.text_units(),
            ["shipping", "late delivery", "from:literal", "future:syntax"]
        );
        assert_eq!(q.phrases(), ["late delivery", "from:literal"]);
        assert_eq!(
            q.semantic_text(),
            "shipping late delivery from:literal future:syntax"
        );
        let q = MailboxQuery::parse(r#""quote \"inside\"" subject:"two words""#).unwrap();
        assert_eq!(q.phrases(), ["quote \"inside\""]);
        assert_eq!(q.subject.as_deref(), Some("two words"));
    }
    #[test]
    fn invalid_input_never_weakens_a_filter() {
        for q in [
            "from:",
            "to:",
            "subject:",
            "folder:",
            "after:tomorrow",
            "before:2026-02-30",
            "\"unfinished",
            "text\0",
            r#""unfinished\"#,
        ] {
            assert!(MailboxQuery::parse(q).is_err(), "{q:?}");
        }
        assert!(MailboxQuery::parse(&"a".repeat(4097)).is_err());
        assert!(MailboxQuery::parse("").unwrap().text.is_empty());
    }
}
