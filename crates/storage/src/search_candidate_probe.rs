//! Bounded planning hints, never coverage or absence proofs. Whichever unit
//! drives candidate discovery, every unit and strict filter is still verified.
use super::*;

pub(super) fn sqlite_match(account: &str, variants: &[String]) -> String {
    sqlite_field_match(account, variants, None)
}
pub(super) fn sqlite_field_match(
    account: &str,
    variants: &[String],
    field: Option<usize>,
) -> String {
    format!(
        "{} AND ({})",
        crate::lexical_text::native_scope(account),
        variants
            .iter()
            .flat_map(|variant| native_units(variant, field)
                .into_iter()
                .map(|native| format!("\"{native}\"")))
            .collect::<Vec<_>>()
            .join(" OR ")
    )
}
pub(super) fn postgres_match(account: &str, unit: &str) -> String {
    postgres_field_match(account, unit, None)
}
pub(super) fn postgres_field_match(account: &str, unit: &str, field: Option<usize>) -> String {
    format!(
        "{} & ({})",
        crate::lexical_text::native_scope(account),
        postgres_unit(unit, field, false)
    )
}
pub(super) fn postgres_phrase(unit: &str, field: Option<usize>) -> String {
    postgres_unit(unit, field, true)
}
fn fields(field: Option<usize>) -> impl Iterator<Item = usize> {
    (0..crate::lexical_text::WEIGHTS.len())
        .filter(move |candidate| field.is_none_or(|field| field == *candidate))
}
fn native_units(unit: &str, field: Option<usize>) -> Vec<String> {
    let mut units: Vec<_> = fields(field)
        .map(|field| crate::lexical_text::field_unit(unit, field))
        .collect();
    if field.is_none() {
        units.push(crate::lexical_text::native_unit(unit));
    }
    units
}
fn postgres_unit(unit: &str, field: Option<usize>, phrase: bool) -> String {
    native_units(unit, field)
        .into_iter()
        .map(|native| {
            format!(
                "({})",
                native
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(if phrase { " <-> " } else { " & " })
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(super) async fn anchor(
    db: &SearchDatabase,
    account: &str,
    alternatives: &[Vec<String>],
) -> Result<Option<usize>> {
    if alternatives.is_empty() {
        return Ok(None);
    }
    // Query grammar and retrieval remain unchanged for longer searches. Bound
    // estimation work while also considering a specific term at the end.
    let mut positions: Vec<_> = (0..alternatives.len()).take(16).collect();
    let last = alternatives.len() - 1;
    if !positions.contains(&last) {
        positions.push(last);
    }
    macro_rules! run {
        ($kind:ty,$pool:expr,$sqlite:expr,$bounded:path) => {{
            let mut q = sqlx::QueryBuilder::<$kind>::new("");
            for (number, position) in positions.iter().enumerate() {
                if number > 0 {
                    q.push(" UNION ALL ");
                }
                q.push("SELECT CAST(")
                    .push(position.to_string())
                    .push(" AS BIGINT),COUNT(*) FROM (SELECT ");
                if $sqlite {
                    q.push("rowid FROM lexical_terms WHERE lexical_terms MATCH ")
                        .push_bind(sqlite_match(account, &alternatives[*position]));
                } else {
                    q.push("id FROM lexical_chunks c WHERE (");
                    for (i, variant) in alternatives[*position].iter().enumerate() {
                        if i > 0 {
                            q.push(" OR ");
                        }
                        q.push("c.lexemes @@ to_tsquery('simple',")
                            .push_bind(postgres_match(account, variant))
                            .push(")");
                    }
                    q.push(")");
                }
                q.push(" LIMIT 1025) probe");
            }
            let mut read = $bounded($pool).await?;
            q.build_query_as::<(i64, i64)>()
                .fetch_all(read.connection())
                .await?
        }};
    }
    let counts = match db {
        SearchDatabase::Sqlite(pool) => run!(sqlx::Sqlite, pool, true, budget::sqlite),
        SearchDatabase::Postgres(pool) => run!(sqlx::Postgres, pool, false, budget::postgres),
    };
    let (position, count) = counts
        .into_iter()
        .min_by_key(|(position, count)| (*count, *position))
        .ok_or(Error::Invalid)?;
    if count >= 1025 {
        Ok(None)
    } else {
        Ok(Some(usize::try_from(position).map_err(|_| Error::Invalid)?))
    }
}
