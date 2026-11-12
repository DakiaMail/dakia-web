//! Offline packaged policy check using synthetic mail only.
use dakia_domain::{DetectedType, InboxKind};
use dakia_inference::{EmailClassificationInput, LocalEmailClassifier};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut classifier = LocalEmailClassifier;
    let messages = [
        EmailClassificationInput::new(
            Some("Example Sender"),
            "sender@example.test",
            "Meeting tomorrow",
            "Can we meet tomorrow?",
            "",
        ),
        EmailClassificationInput::new(
            None,
            "service@example.test",
            "Your verification code: 123456",
            "Valid for 5 minutes",
            "",
        ),
    ];
    let results = classifier.classify(&messages)?;
    if results.len() != 2
        || results[0].kind != InboxKind::Personal
        || results[1].kind != InboxKind::Automated
        || !results[1].types.contains(&DetectedType::VerificationCode)
    {
        return Err("classification policy check failed".into());
    }
    println!("Deterministic classification policy verified");
    Ok(())
}
