//! Bounded offline detection of the displayed body, independent of classification.
use scraper::{ElementRef, Html};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, sync::Mutex};
#[cfg(test)]
#[path = "language_browser_tests.rs"]
mod browser_tests;

#[derive(Default)]
pub(crate) struct Detector(Mutex<VecDeque<([u8; 32], Option<String>)>>);
impl Detector {
    pub(crate) fn detect(&self, account: &str, id: &str, text: &str, html: &str) -> Option<String> {
        // Retain bounded digests/results, never additional copies of mail.
        let mut hash = Sha256::new();
        for part in [account, id, text, html] {
            hash.update(part.len().to_le_bytes());
            hash.update(part.as_bytes());
        }
        let key = hash.finalize().into();
        if let Some((_, value)) = self.0.lock().unwrap().iter().find(|(k, _)| *k == key) {
            return value.clone();
        }
        let result = detect_body(text, html);
        let mut cache = self.0.lock().unwrap();
        if cache.len() == 256 {
            cache.pop_front();
        }
        cache.push_back((key, result.clone()));
        result
    }
}

fn boilerplate(line: &str) -> bool {
    let line = line.trim().to_lowercase();
    [
        "unsubscribe",
        "loobu tellimusest",
        "you received this email",
        "you are receiving this",
        "this email is intended",
        "this message is confidential",
        "all rights reserved",
        "manage your preferences",
        "view in browser",
        "view this email in your browser",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
        || line.starts_with('©')
        || (line.starts_with("on ") && line.ends_with("wrote:"))
}

pub(crate) fn detect(text: &str) -> Option<String> {
    let bounded: String = text.chars().take(10_000).collect();
    let mut sample = String::new();
    let mut letters = 0;
    for line in bounded.lines() {
        if line.trim_start().starts_with('>') {
            continue;
        }
        if boilerplate(line) {
            // Browser links at the start are not the end of the message.
            if letters >= 48 {
                break;
            }
            continue;
        }
        letters += line.chars().filter(|c| c.is_alphabetic()).count();
        sample.push_str(line);
        sample.push(' ');
    }
    if sample.chars().filter(|c| c.is_alphabetic()).count() < 48 {
        return None;
    }
    let info = whatlang::detect(&sample)?;
    if !info.is_reliable() || info.confidence() < 0.7 {
        return None;
    }
    // The translation registry uses ISO-639-1; whatlang returns ISO-639-3.
    let code = match info.lang().code() {
        "eng" => "en",
        "est" => "et",
        "ara" => "ar",
        "cmn" => "zh",
        "jpn" => "ja",
        "bul" => "bg",
        "ben" => "bn",
        "cat" => "ca",
        "ces" => "cs",
        "dan" => "da",
        "deu" => "de",
        "ell" => "el",
        "spa" => "es",
        "pes" => "fa",
        "fin" => "fi",
        "fra" => "fr",
        "guj" => "gu",
        "heb" => "he",
        "hin" => "hi",
        "hrv" => "hr",
        "hun" => "hu",
        "ind" => "id",
        "ita" => "it",
        "kan" => "kn",
        "kor" => "ko",
        "lit" => "lt",
        "lav" => "lv",
        "mal" => "ml",
        "nob" => "nb",
        "nld" => "nl",
        "pol" => "pl",
        "por" => "pt",
        "ron" => "ro",
        "rus" => "ru",
        "slk" => "sk",
        "slv" => "sl",
        "srp" => "sr",
        "swe" => "sv",
        "tam" => "ta",
        "tel" => "te",
        "tha" => "th",
        "tur" => "tr",
        "ukr" => "uk",
        "vie" => "vi",
        _ => return None,
    };
    Some(code.to_owned())
}

fn excluded(element: ElementRef<'_>) -> bool {
    let value = element.value();
    if matches!(
        value.name(),
        "head"
            | "script"
            | "style"
            | "template"
            | "noscript"
            | "svg"
            | "blockquote"
            | "footer"
            | "nav"
    ) || value.attr("hidden").is_some()
        || value.attr("aria-hidden") == Some("true")
    {
        return true;
    }
    let style = value
        .attr("style")
        .unwrap_or("")
        .to_lowercase()
        .replace(char::is_whitespace, "");
    if style
        .split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .any(|(key, value)| {
            let value = value.trim_end_matches("!important");
            matches!(
                (key, value),
                ("display", "none")
                    | ("visibility", "hidden")
                    | ("mso-hide", "all")
                    | ("opacity", "0" | "0.0")
                    | ("font-size", "0" | "0px" | "0pt" | "0em")
            )
        })
    {
        return true;
    }
    let labels = format!(
        "{} {}",
        value.attr("class").unwrap_or(""),
        value.attr("id").unwrap_or("")
    )
    .to_lowercase();
    labels.split(|c: char| !c.is_alphanumeric()).any(|part| {
        matches!(
            part,
            "preheader" | "preview" | "footer" | "unsubscribe" | "legal" | "signature"
        )
    }) || labels.contains("gmail_quote")
        || labels.contains("yahoo_quoted")
        || labels.contains("moz-cite")
}

fn detect_body(text: &str, html: &str) -> Option<String> {
    if html.trim().is_empty() {
        return detect(text);
    }
    // Use the displayed HTML, not a conflicting MIME text alternative.
    let bounded: String = html.chars().take(256_000).collect();
    let document = Html::parse_document(&bounded);
    let mut sample = String::new();
    let mut previous_block = None;
    for node in document.tree.root().descendants().take(16_384) {
        let Some(value) = node.value().as_text() else {
            continue;
        };
        let ancestors: Vec<_> = node
            .ancestors()
            .filter_map(ElementRef::wrap)
            .take(65)
            .collect();
        if ancestors.len() > 64 {
            return None;
        }
        if ancestors.iter().any(|e| excluded(*e)) {
            continue;
        }
        let block = ancestors
            .iter()
            .find(|e| {
                matches!(
                    e.value().name(),
                    "p" | "div" | "td" | "li" | "h1" | "h2" | "h3" | "section" | "body"
                )
            })
            .map(|e| e.id());
        if block != previous_block {
            sample.push('\n');
            previous_block = block;
        }
        sample.push_str(value);
        sample.push(' ');
        if sample.len() > 40_000 {
            break;
        }
    }
    detect(&sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    pub(super) struct Fixture {
        pub(super) name: String,
        pub(super) text: String,
        pub(super) html: String,
        pub(super) expected: Option<String>,
    }
    pub(super) fn fixtures() -> Vec<Fixture> {
        serde_json::from_str(include_str!(
            "../../../apps/web/browser/fixtures/translation-bodies.json"
        ))
        .unwrap()
    }
    #[test]
    fn displayed_body_fixtures_and_scoped_revision_cache() {
        let detector = Detector::default();
        for body in fixtures() {
            assert_eq!(
                detector.detect("a", "same-id", &body.text, &body.html),
                body.expected,
                "cold {}",
                body.name
            );
            assert_eq!(
                detector.detect("a", "same-id", &body.text, &body.html),
                body.expected,
                "warm {}",
                body.name
            );
            assert_eq!(
                detector.detect("b", "same-id", &body.text, &body.html),
                body.expected,
                "account {}",
                body.name
            );
        }
        for i in 0..300 {
            detector.detect("a", &i.to_string(), "", "");
        }
        assert_eq!(detector.0.lock().unwrap().len(), 256);
        for body in fixtures() {
            assert_eq!(
                detector.detect("a", "same-id", &body.text, &body.html),
                body.expected
            );
        }
    }
    #[test]
    fn body_detection_timing() {
        use std::time::Instant;
        for body in fixtures() {
            let detector = Detector::default();
            let start = Instant::now();
            let result = detector.detect("timing", &body.name, &body.text, &body.html);
            let cold = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            for _ in 0..100 {
                assert_eq!(
                    detector.detect("timing", &body.name, &body.text, &body.html),
                    result
                );
            }
            println!(
                "{} cold_ms={cold:.4} warm_mean_ms={:.4}",
                body.name,
                start.elapsed().as_secs_f64() * 10.0
            );
        }
    }
    #[test]
    fn english_main_is_not_overridden_by_foreign_footer() {
        let body = "Please review the attached meeting notes before our discussion tomorrow. We will confirm the final project schedule and answer any remaining questions together.\n\nLoobu tellimusest. See kiri on mõeldud ainult adressaadile. Palun kustutage see kiri, kui saite selle ekslikult.";
        assert_eq!(detect(body), Some("en".into()));
    }
    #[test]
    fn browser_preamble_does_not_discard_the_actual_body() {
        let body = fixtures()
            .into_iter()
            .find(|f| f.expected.as_deref() == Some("et"))
            .unwrap();
        assert_eq!(
            detect_body(
                "",
                &format!("<a>View this email in your browser</a>{}", body.html)
            ),
            Some("et".into())
        );
    }
    #[test]
    fn fractional_visible_styles_are_not_hidden_preheaders() {
        let body = fixtures()
            .into_iter()
            .find(|f| f.expected.as_deref() == Some("et"))
            .unwrap();
        assert_eq!(
            detect_body(
                "",
                &format!(
                    "<div style='opacity:0.9;font-size:0.9em'>{}</div>",
                    body.html
                )
            ),
            Some("et".into())
        );
    }
    #[test]
    fn reliable_english_only_not_short_or_unknown() {
        assert_eq!(detect("Please review the attached meeting notes before our discussion tomorrow. We will confirm the final project schedule and answer any remaining questions together."), Some("en".into()));
        for sample in [
            "",
            "OK",
            "Hello world",
            "123456 !!!",
            "Tere!",
            "会议明天开始",
        ] {
            assert_eq!(detect(sample), None, "{sample}");
        }
        assert_ne!(detect("Tere hommikust! Palun vaadake enne homset koosolekut läbi lisatud dokumendid. Me arutame projekti järgmisi samme ja vastame kõikidele küsimustele."), Some("en".into()));
        assert_ne!(detect("يرجى مراجعة المستندات المرفقة قبل اجتماع الغد. سوف نناقش الخطوات التالية للمشروع ونجيب على جميع الأسئلة المتبقية."), Some("en".into()));
    }
    #[test]
    fn ignores_quoted_history_and_retains_controls_for_mixed_text() {
        assert_eq!(detect("Tere!\n> Please review the attached meeting notes before our discussion tomorrow. We will confirm the final project schedule together."), None);
        let mixed = format!("Tere hommikust! Palun vaadake enne homset koosolekut läbi lisatud dokumendid.\n{}", "This email is intended for the named recipient only. Please contact our support team if you have received this message by mistake. ".repeat(8));
        assert_ne!(detect(&mixed), Some("en".into()));
    }
}
