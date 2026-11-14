//! Offline packaged CPU check. Uses only synthetic text and local model assets.
use dakia_inference::embedding::LocalSearchEmbedder;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("DAKIA_SEARCH_MODEL_DIR")
        .unwrap_or_else(|_| "crates/inference/assets/multilingual-e5-small".into());
    let mut model = LocalSearchEmbedder::from_dir(path)?;
    let passages = model.embed_passages(&[
        "Your package is scheduled for delivery tomorrow.",
        "A recipe for baking bread.",
    ])?;
    let query = model.embed_query("When will the parcel arrive?")?;
    let score = |v: &[f32]| v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>();
    if score(&passages[0]) <= score(&passages[1]) {
        return Err("search model semantic smoke failed".into());
    }
    dakia_storage::check_search_accelerator(&query, &passages[0], &passages[1])?;
    println!("Pinned search model and vector accelerator ranked synthetic passages on CPU");
    if std::env::args().any(|arg| arg == "--timing") {
        let mut samples = Vec::with_capacity(100);
        for n in 0..100 {
            let query = format!("Where is package number {n} and when will it arrive?");
            let start = std::time::Instant::now();
            let _ = model.embed_query(&query)?;
            samples.push(start.elapsed().as_secs_f64() * 1000.);
        }
        samples.sort_by(f64::total_cmp);
        println!("{{\"scope\":\"CPU query embedding only\",\"samples\":100,\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"max_ms\":{:.3}}}",samples[49],samples[94],samples[99]);
    }
    Ok(())
}
