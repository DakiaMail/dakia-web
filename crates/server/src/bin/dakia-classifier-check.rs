//! Offline packaged-runtime check using only a synthetic message.
use dakia_inference::{EmailClassificationInput, LocalEmailClassifier};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("DAKIA_CLASSIFIER_DIR")
        .unwrap_or_else(|_| "crates/inference/assets/email-classifier-v2".into());
    let mut classifier = LocalEmailClassifier::from_dir(path)?;
    let result = classifier.classify(&[EmailClassificationInput::new(
        Some("Example Sender"),
        "sender@example.test",
        "Meeting tomorrow",
        "Can we meet tomorrow at ten?",
        "",
    )])?;
    if result.len() != 1 {
        return Err("classifier did not return one result".into());
    }
    println!("Bundled classifier loaded and classified a synthetic message");
    Ok(())
}
