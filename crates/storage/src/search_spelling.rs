//! Bounded account-scoped spelling lookup. Durable deletion signatures select
//! candidates; shared Unicode edit checks define matching on both databases.
use super::*;
use unicode_general_category::{get_general_category, GeneralCategory as G};
const CANDIDATES: i64 = 512;
const ALTERNATIVES: usize = 8;
const EXPANDED_UNITS: usize = 16;
fn ordinary(raw: &str) -> Option<String> {
    let unit = crate::lexical_text::unit(raw).ok()??;
    let word = unit.strip_prefix('w')?;
    let count = word.chars().count();
    if !(4..=32).contains(&count)
        || word.chars().filter(|c| c.is_alphabetic()).count() < 4
        || !word.chars().all(|c| {
            c.is_alphabetic()
                || matches!(
                    get_general_category(c),
                    G::NonspacingMark | G::SpacingMark | G::EnclosingMark
                )
        })
        || !raw.chars().all(|c| {
            c.is_alphabetic()
                || matches!(
                    get_general_category(c),
                    G::NonspacingMark | G::SpacingMark | G::EnclosingMark
                )
        })
    {
        return None;
    }
    Some(word.into())
}
fn signatures(word: &str) -> Vec<String> {
    let chars: Vec<_> = word.chars().collect();
    let mut keys = std::collections::BTreeSet::from([word.to_owned()]);
    for skip in 0..chars.len() {
        keys.insert(
            chars
                .iter()
                .enumerate()
                .filter_map(|(i, c)| (i != skip).then_some(*c))
                .collect(),
        );
    }
    keys.into_iter().collect()
}
fn one_edit(a: &str, b: &str) -> bool {
    let a: Vec<_> = a.chars().collect();
    let b: Vec<_> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let i = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    if i == a.len().min(b.len()) {
        return true;
    }
    if a.len() == b.len() {
        a[i + 1..] == b[i + 1..]
            || (i + 1 < a.len() && a[i] == b[i + 1] && a[i + 1] == b[i] && a[i + 2..] == b[i + 2..])
    } else if a.len() < b.len() {
        a[i..] == b[i + 1..]
    } else {
        a[i + 1..] == b[i..]
    }
}
impl SearchDatabase {
    pub async fn spelling(
        &self,
        account: &str,
        units: &[String],
        allowed: &[bool],
    ) -> Result<Vec<Vec<String>>> {
        if units.len() != allowed.len() || units.len() > 2048 {
            return Err(Error::Invalid);
        }
        let mut expanded = 0;
        let mut result = Vec::with_capacity(units.len());
        for (raw, allowed) in units.iter().zip(allowed) {
            let mut alternatives = vec![raw.clone()];
            if *allowed && expanded < EXPANDED_UNITS {
                if let Some(word) = ordinary(raw) {
                    expanded += 1;
                    let keys = serde_json::to_string(&signatures(&word))?;
                    let mut candidates: Vec<(String,i64)> = match self {
                        Self::Sqlite(db) => sqlx::query_as("SELECT w.word,w.refs FROM spelling_words w JOIN (SELECT DISTINCT word FROM spelling_signatures WHERE account_id=? AND signature IN (SELECT value FROM json_each(?)) ORDER BY word LIMIT ?) c ON c.word=w.word WHERE w.account_id=?")
                            .bind(account).bind(&keys).bind(CANDIDATES).bind(account).fetch_all(db).await?,
                        // Keep the same bounded candidate prefix as SQLite. pg_trgm
                        // rejects unrelated ASCII signatures; Unicode uses the shared
                        // edit check as well, independent of server locale categories.
                        Self::Postgres(db) => sqlx::query_as("WITH candidates AS MATERIALIZED (SELECT DISTINCT word COLLATE \"C\" AS word FROM spelling_signatures WHERE account_id=$1 AND signature IN (SELECT jsonb_array_elements_text($2::jsonb)) ORDER BY word LIMIT $4) SELECT w.word,w.refs FROM candidates c JOIN spelling_words w ON w.account_id=$1 AND w.word=c.word WHERE similarity(w.word,$3)>0 OR octet_length(w.word)<>length(w.word)")
                            .bind(account).bind(&keys).bind(&word).bind(CANDIDATES).fetch_all(db).await?,
                    };
                    candidates.retain(|(candidate, _)| {
                        candidate != &word
                            && ordinary(candidate).as_deref() == Some(candidate.as_str())
                            && one_edit(&word, candidate)
                    });
                    candidates.sort_by(|(a, af), (b, bf)| bf.cmp(af).then_with(|| a.cmp(b)));
                    alternatives.extend(
                        candidates
                            .into_iter()
                            .take(ALTERNATIVES)
                            .map(|(word, _)| word),
                    );
                }
            }
            result.push(alternatives);
        }
        Ok(result)
    }
}
impl SearchConnection {
    pub async fn spelling(
        &self,
        account: &str,
        units: &[String],
        allowed: &[bool],
    ) -> Result<Vec<Vec<String>>> {
        self.connection()
            .await?
            .spelling(account, units, allowed)
            .await
    }
}
impl crate::Store {
    pub async fn search_spelling(
        &self,
        a: &crate::AuthorizedAccount,
        units: &[String],
        allowed: &[bool],
    ) -> Result<Vec<Vec<String>>> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        if units.len() != allowed.len() || units.len() > 2048 {
            return Err(Error::Invalid);
        }
        if !units
            .iter()
            .zip(allowed)
            .any(|(word, enabled)| *enabled && ordinary(word).is_some())
        {
            return Ok(units.iter().map(|unit| vec![unit.clone()]).collect());
        }
        self.search_database
            .as_ref()
            .ok_or(Error::Invalid)?
            .spelling(a.id(), units, allowed)
            .await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_edits_and_non_words_have_fixed_bounds() {
        for (a, b) in [
            ("parcel", "parcle"),
            ("parcel", "parsel"),
            ("parcel", "parcl"),
            ("parcel", "parccel"),
            ("東京大阪", "東京大坂"),
            ("доставка", "достаква"),
        ] {
            assert!(one_edit(a, b));
            assert!(signatures(a).iter().any(|key| signatures(b).contains(key)));
        }
        assert!(!one_edit("parcel", "parxyz"));
        for raw in [
            "abc",
            "1234",
            "ab12cd",
            "a@example.invalid",
            "two words",
            "can't",
            "🙂🙂🙂🙂",
        ] {
            assert!(ordinary(raw).is_none(), "{raw}");
        }
        assert_eq!(ordinary("CAFÉ"), Some("cafe".into()));
        assert!(ordinary("東京大阪").is_some());
    }
}
