//! A date-ordered head is complete only after every admitted row reaches a
//! proven global score ceiling. Otherwise use the general, unbounded ranker.
use super::*;

// Presence establishes a safe field ceiling without scanning complete posting lists.
macro_rules! field_probe {
    ($q:ident,$sqlite:expr,$account:expr,$number:expr,$unit:expr,$field:expr,$limit:expr)=>{{
        let field=$field;
        $q.push("SELECT CAST(").push($number.to_string()).push(" AS BIGINT),CAST(").push(field.to_string()).push(" AS BIGINT),COUNT(CASE WHEN ordinal=0 THEN 1 END) FROM (SELECT c.ordinal FROM ");
        if $sqlite {
            $q.push("lexical_terms CROSS JOIN lexical_chunks c INDEXED BY lexical_bound_lookup ON c.account_id=").push_bind($account).push(" AND c.id=lexical_terms.rowid WHERE ");
        } else { $q.push("lexical_chunks c WHERE "); }
        native_field_match!($q,$sqlite,$account,$unit,Some(field));
        $q.push(" AND c.account_id=").push_bind($account).push(" AND c.revision=").push_bind(crate::lexical_text::REVISION)
            .push(" AND c.field=").push(field.to_string()).push(" LIMIT ").push_bind($limit)
            .push(") sample HAVING COUNT(*)>0");
    }};
}

#[cfg(test)]
pub(super) async fn single_unit_head(
    db: &SearchDatabase,
    account: &str,
    variants: &[String],
    exact: &str,
    intrinsic_phrase: bool,
    limit: i64,
    predicates: Option<(&CachedSearchFilter<'_>, &str)>,
) -> Result<Option<Vec<(String, String, f64)>>> {
    ranked_head(
        db,
        account,
        &[variants.to_vec()],
        exact,
        variants
            .first()
            .filter(|_| intrinsic_phrase)
            .map(String::as_str),
        limit,
        predicates,
    )
    .await
}
pub(super) async fn ranked_head(
    db: &SearchDatabase,
    account: &str,
    alternatives: &[Vec<String>],
    exact: &str,
    phrase: Option<&str>,
    limit: i64,
    predicates: Option<(&CachedSearchFilter<'_>, &str)>,
) -> Result<Option<Vec<(String, String, f64)>>> {
    // Bound optimizer work only. Longer public queries retain the full ranker.
    if exact.len() > 256
        || alternatives.is_empty()
        || alternatives.len() > 16
        || alternatives
            .iter()
            .any(|variants| variants.is_empty() || variants.len() > 9)
    {
        return Ok(None);
    }
    let mut probes = vec![];
    let mut groups = vec![];
    for variants in alternatives {
        let mut group = vec![];
        for (variant, unit) in variants.iter().enumerate() {
            group.push((probes.len(), i64::from(variant > 0)));
            probes.push(unit.as_str());
        }
        groups.push(group);
    }
    let phrase_probe = phrase.map(|phrase| {
        // Reuse the exact phrase probe when it is already a complete unit.
        if let Some(position) = probes.iter().position(|unit| *unit == phrase) {
            position
        } else {
            let position = probes.len();
            probes.push(phrase);
            position
        }
    });
    macro_rules! run {
        ($kind:ty,$pool:expr,$sqlite:expr,$snapshot:path)=>{{
            let mut read=$snapshot($pool).await?;
            let mut timing=crate::timing::SearchTiming::new("bound_profile");
            let profile:Option<(i64,i64,i64)>=sqlx::query_as(if $sqlite {
                "SELECT CAST(revision AS BIGINT),sweep_done,CAST(pending AS BIGINT) FROM lexical_accounts WHERE account_id=?"
            }else{
                "SELECT CAST(revision AS BIGINT),sweep_done,CAST(pending AS BIGINT) FROM lexical_accounts WHERE account_id=$1"
            }).bind(account).fetch_optional(read.connection()).await?;
            if !profile.is_some_and(|(revision,done,pending)|revision==crate::lexical_text::REVISION && done!=0 && pending==0) {
                return Ok(None);
            }
            timing.stage("bound_subject");
            // A possible exact-subject bonus needs a separate ranking stratum.
            // Keep that case on the general path for now. Full equality guards
            // the PostgreSQL hash lookup, including hypothetical collisions.
            // Fence the hash prefilter so equality cannot choose the trigram index.
            let exact_subject:bool=sqlx::query_scalar(if $sqlite {
                "SELECT EXISTS(SELECT 1 FROM documents INDEXED BY documents_subject_identity WHERE account_id=? AND subject=? AND state<>'deleted')"
            }else{
                "SELECT EXISTS(SELECT 1 FROM (SELECT subject FROM documents WHERE account_id=$1 AND md5(subject)=md5($2) AND state<>'deleted' OFFSET 0) matched WHERE subject=$2)"
            }).bind(account).bind(exact).fetch_one(read.connection()).await?;
            if exact_subject { return Ok(None); }
            if $sqlite {
                if let Some((filter,_))=predicates {
                    for (column,index,value) in [
                        ("sender","documents_sender_identity",filter.from),
                        ("recipients","documents_recipients_identity",filter.to),
                        ("subject","documents_subject_identity",filter.subject)
                    ] {
                        let Some(value)=value else {continue;};
                        timing.stage("bound_metadata_hint");
                        let sql=format!("SELECT EXISTS(SELECT 1 FROM documents INDEXED BY {index} WHERE account_id=? AND state<>'deleted' AND instr({column},?)>0)");
                        #[cfg(test)] {
                            let plan=sqlx::query_as::<$kind,(i64,i64,i64,String)>(&format!("EXPLAIN QUERY PLAN {sql}"))
                                .bind(account).bind(normalize_search(value)).fetch_all(read.connection()).await?;
                            assert!(plan.iter().any(|row|row.3.contains(&format!("INDEX {index} (account_id=?)"))),"{plan:?}");
                            // SQLite labels the existing partial subject index
                            // without COVERING, even when no table column is read.
                            // Check the VM too so every hint stays payload-free.
                            type Instruction=(i64,String,i64,i64,i64,Option<String>,i64,Option<String>);
                            let root:i64=sqlx::query_scalar("SELECT rootpage FROM sqlite_schema WHERE type='table' AND name='documents'")
                                .fetch_one(read.connection()).await?;
                            let code=sqlx::query_as::<$kind,Instruction>(&format!("EXPLAIN {sql}"))
                                .bind(account).bind(normalize_search(value)).fetch_all(read.connection()).await?;
                            let cursors:std::collections::HashSet<_>=code.iter().filter(|row|row.1=="OpenRead" && row.3==root).map(|row|row.2).collect();
                            assert!(!code.iter().any(|row|row.1=="Column" && cursors.contains(&row.2)),"metadata hint read a table payload: {code:?}");
                        }
                        if !sqlx::query_scalar::<$kind,bool>(&sql).bind(account).bind(normalize_search(value)).fetch_one(read.connection()).await? {
                            // A hint only declines this plan. The complete ranker
                            // still establishes the answer, including absence.
                            return Ok(None);
                        }
                    }
                }
            }
            timing.stage("bound_fields");

            let mut q=sqlx::QueryBuilder::<$kind>::new("");
            let mut first=true;
            for (number,unit) in probes.iter().enumerate() {
                for field in 0..crate::lexical_text::WEIGHTS.len() {
                    if !first { q.push(" UNION ALL "); } first=false;
                    field_probe!(q,$sqlite,account,number,*unit,field,1_i64);
                }
            }
            let fields=q.build_query_as::<(i64,i64,i64)>().fetch_all(read.connection()).await?;
            if fields.iter().any(|(_,field,_)| !(0..6).contains(field)) {return Ok(None);}
            let mut strata:Vec<Vec<(usize,i64)>>=vec![];
            let mut ceiling=0;
            for group in &groups {
                let hits:Vec<_>=fields.iter().filter_map(|(probe,field,count)| {
                    group.iter().find(|(position,_)|*position==*probe as usize)
                        .map(|(_,penalty)|(*probe as usize,*field,*count,crate::lexical_text::WEIGHTS[*field as usize]-penalty))
                }).collect();
                if hits.iter().any(|(_,field,_,_)|matches!(field,0|1|4)) {
                    // A possible exact-participant bonus is another stratum.
                    return Ok(None);
                }
                let Some(maximum)=hits.iter().map(|(_,_,_,weight)|*weight).max() else {return Ok(None);};
                let best:Vec<_>=hits.into_iter().filter(|(_,_,_,weight)|*weight==maximum).collect();
                ceiling+=maximum;
                strata.push(best.into_iter().map(|(probe,field,_,_)|(probe,field)).collect());
            }
            if let Some(probe)=phrase_probe {
                let hits:Vec<_>=fields.iter().filter(|(position,_,_)|*position as usize==probe).collect();
                if let Some(maximum)=hits.iter().map(|(_,field,_)|crate::lexical_text::PHRASE_WEIGHTS[*field as usize]).max() {
                    let best:Vec<_>=hits.into_iter().filter(|(_,field,_)|crate::lexical_text::PHRASE_WEIGHTS[*field as usize]==maximum).collect();
                    ceiling+=maximum;
                    let stratum:Vec<_>=best.into_iter().map(|(_,field,_)|(probe,*field)).collect();
                    // A phrase proves a unit only in every selected phrase field, with exact token boundaries.
                    let phrase_text=format!(" {} ",probes[probe]);
                    strata.retain(|units|!stratum.iter().all(|(_,field)|units.iter().any(|(unit,unit_field)|
                        field==unit_field && phrase_text.contains(&format!(" {} ",probes[*unit])))));
                    strata.push(stratum);
                }
            }
            // The complete-sized head proves its own density; sparse units already use the native anchor.
            timing.stage("bound_head");

            // Only the optional proof scans this prefix. Sparse intersections
            // decline to the complete ranker instead of exhausting its budget.
            let prefix_limit=limit.saturating_mul(16).max(1024);
            // A tuple boundary lets ordered index scans stop as soon as the proof is complete.
            let boundary:Option<(String,String)>=sqlx::query_as(if $sqlite {
                "SELECT received_at,message_id FROM documents INDEXED BY documents_filter_order WHERE account_id=? AND state<>'deleted' ORDER BY received_at DESC,message_id DESC LIMIT 1 OFFSET ?"
            }else{
                "SELECT received_at,message_id FROM documents WHERE account_id=$1 AND state<>'deleted' ORDER BY received_at DESC,message_id COLLATE \"C\" DESC LIMIT 1 OFFSET $2"
            }).bind(account).bind(prefix_limit-1).fetch_optional(read.connection()).await?;
            let mut q=sqlx::QueryBuilder::<$kind>::new("");
            if $sqlite { q.push("SELECT COALESCE(json_group_array(json_array(metadata,source,score)),'[]') FROM ("); }
            q.push("SELECT d.metadata,d.source,CAST(").push_bind(-(ceiling as f64))
                .push(if $sqlite {" AS REAL) AS score FROM documents d INDEXED BY documents_filter_order WHERE d.account_id="} else {" AS DOUBLE PRECISION) AS score FROM documents d WHERE d.account_id="})
                .push_bind(account).push(" AND d.state<>'deleted'");
            if let Some((date,id))=&boundary {
                q.push(" AND (d.received_at,").push(if $sqlite {"d.message_id"}else{"d.message_id COLLATE \"C\""})
                    .push(")>=(").push_bind(date).push(",").push_bind(id).push(")");
            }
            if let Some((filter,at))=predicates {
                // An account-wide ceiling also bounds every strict subset.
                // Filters must be applied before admitting the full-sized head;
                // a sparse high-scoring stratum falls back to the full ranker.
                let at=at.to_owned();
                metadata_predicates!(q,$sqlite,filter,"d",at);
            }
            for best in &strata {
                let subject_only=best.iter().all(|(_,field)|*field==2);
                q.push(" AND EXISTS(SELECT 1 FROM lexical_chunks c ");
                if $sqlite { q.push(if subject_only {"INDEXED BY lexical_subject_head "} else {"INDEXED BY lexical_field_lookup "}); }
                q.push("WHERE c.account_id=d.account_id AND c.message_id=d.message_id AND c.content_version=d.content_version AND c.revision=")
                    .push_bind(crate::lexical_text::REVISION).push(" AND c.native_text<>'' AND c.field IN (");
                let selected:std::collections::BTreeSet<_>=best.iter().map(|(_,field)|*field).collect();
                for (i,field) in selected.into_iter().enumerate() {
                    if i>0 { q.push(","); } q.push(field.to_string());
                }
                q.push(") AND (");
                for (i,(probe,field)) in best.iter().enumerate() {
                    if i>0 { q.push(" OR "); }
                    q.push("(c.field=").push(field.to_string()).push(if $sqlite {" AND instr(' '||c.text||' ',"} else {" AND strpos(' '||c.text||' ',"})
                        .push_bind(format!(" {} ",probes[*probe])).push(")>0)");
                }
                // Keep fresh-account estimates from turning identity lookups into repeated account scans.
                q.push(if $sqlite {"))"} else {") OFFSET 0)"});
            }
            q.push(" ORDER BY d.received_at DESC,").push(if $sqlite {"d.message_id"} else {"d.message_id COLLATE \"C\""})
                .push(" DESC LIMIT ").push_bind(limit);
            if $sqlite { q.push(")"); }
            #[cfg(test)]
            if $sqlite {
                let sql=format!("EXPLAIN QUERY PLAN {}",q.sql());
                let mut explain=sqlx::query_as::<$kind,(i64,i64,i64,String)>(&sql);
                for _ in 0..q.sql().matches('?').count() { explain=explain.bind(None::<String>); }
                let plan=explain.fetch_all(read.connection()).await?;
                assert!(plan.iter().any(|row|row.3.contains("documents_filter_order (account_id=?")),"{plan:?}");
                let subject_only=strata.iter().all(|best|best.iter().all(|(_,field)|*field==2));
                let lookup=if subject_only {"lexical_subject_head (account_id=? AND message_id=? AND field=?"} else {"lexical_field_lookup (account_id=? AND message_id=? AND field=?"};
                assert!(plan.iter().any(|row|row.3.contains(lookup)),"{plan:?}");
                assert!(!plan.iter().any(|row|row.3.contains("TEMP B-TREE") || row.3.contains("MATERIALIZE prefix")),"{plan:?}");
            }
            // Transfer the bounded SQLite head once instead of waking its worker for every row.
            let rows:Vec<(String,String,f64)>=if $sqlite {
                let query=q.build_query_as::<(String,)>();
                #[cfg(feature="test-support")]
                let query=crate::timing::capture_lexical(query)?;
                let (batch,)=query.fetch_one(read.connection()).await?;
                serde_json::from_str(&batch)?
            } else {
                let query=q.build_query_as::<(String,String,f64)>();
                #[cfg(feature="test-support")]
                let query=crate::timing::capture_lexical(query)?;
                query.fetch_all(read.connection()).await?
            };
            timing.stage("bound_proof");
            // Every returned row has the maximum possible score, and all unseen
            // rows are older. Fewer rows cannot establish a complete ranked head.
            Ok((rows.len()==limit as usize).then_some(rows))
        }};
    }
    match db {
        SearchDatabase::Sqlite(pool) => run!(sqlx::Sqlite, pool, true, budget::sqlite_snapshot),
        SearchDatabase::Postgres(pool) => {
            run!(sqlx::Postgres, pool, false, budget::postgres_snapshot)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn contract(url: Option<&str>) {
        let dir = tempfile::tempdir().unwrap();
        let target = SearchDatabase::open(dir.path(), url).await.unwrap();
        let account = uuid::Uuid::new_v4().to_string();
        let mut records = vec![];
        for n in 0..130 {
            let id = format!("bound-{n:03}");
            let date = if n % 2 == 0 {
                "2020-01-01"
            } else {
                "2026-11-01"
            };
            let subject = if n % 2 == 0 {
                "Garden care"
            } else {
                "Other topic"
            };
            let message:Message=serde_json::from_value(serde_json::json!({"id":id,"account_id":account,"thread_id":id,"provider_id":id,"folder_ids":if n%4==0 {vec!["INBOX","Work"]} else {vec!["INBOX"]},"sender":{"email":if n%3==0 {"nora@example.invalid"} else {"maya@example.invalid"},"name":if n%3==0 {"Nora"} else {"Maya"}},"to":if n%4==0 {vec![serde_json::json!({"email":"carbon@example.invalid","name":"Carbon"})]} else {vec![]},"subject":subject,"preview":"","received_at":format!("{date}T00:00:00Z"),"received_date":if n%11==0 {None} else {Some(date)},"unread":n%3==0,"starred":n%5==0,"category":if n%7==0 {None} else if n%4==0 {Some("other")} else {Some("people")},"category_status":"pending","revision":1})).unwrap();
            records.push((
                id.clone(),
                1,
                Some(Document {
                    message,
                    source: id,
                    sender: if n % 3 == 0 {
                        "nora\nnora@example.invalid"
                    } else {
                        "maya\nmaya@example.invalid"
                    }
                    .into(),
                    recipients: if n % 4 == 0 {
                        "carbon\ncarbon@example.invalid"
                    } else {
                        ""
                    }
                    .into(),
                    mime_recipients: String::new(),
                    attachment_details: "[]".into(),
                    body_payload: true,
                    snoozed_until: (n % 3 == 0).then(|| "2099-01-01T00:00:00.000Z".into()),
                    subject: normalize_search(subject),
                    body: format!(
                        "historical correspondence garden {}",
                        if n == 0 { "raretoken" } else { "ordinary" }
                    ),
                    participants: String::new(),
                    attachments: "local extracted attachment".into(),
                    state: "ready".into(),
                }),
            ));
        }
        target.deliver_batch(&account, &records).await.unwrap();
        while target.lexical_turn(&account, true).await.unwrap() {}
        for (raw, variants, intrinsic, expected) in [
            ("garden", vec!["wgarden"], false, true),
            ("gardne", vec!["wgardne", "wgarden"], false, true),
            (
                "historical correspondence",
                vec!["whistorical wcorrespondence"],
                true,
                true,
            ),
            (
                "local extracted attachment",
                vec!["wlocal wextracted wattachment"],
                true,
                true,
            ),
            ("garden care", vec!["wgarden wcare"], true, false),
            ("maya", vec!["wmaya"], false, false),
            ("raretoken", vec!["wraretoken"], false, false),
        ] {
            let canonical: Vec<_> = variants.into_iter().map(str::to_owned).collect();
            let bounded = single_unit_head(&target, &account, &canonical, raw, intrinsic, 51, None)
                .await
                .unwrap();
            assert_eq!(bounded.is_some(), expected, "{raw}");
            let units = vec![raw.to_owned()];
            let expanded = vec![canonical
                .iter()
                .map(|unit| {
                    unit.split_whitespace()
                        .map(|word| word.strip_prefix('w').unwrap())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()];
            let full = target
                .lexical_rows(
                    &account,
                    &CachedSearchFilter {
                        text: Some(raw),
                        text_units: Some(&units),
                        text_alternatives: Some(&expanded),
                        ..Default::default()
                    },
                    LexicalPage {
                        ranked: true,
                        verify_plan: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            if let Some(bounded) = bounded {
                let actual: Vec<_> = bounded
                    .into_iter()
                    .map(|(raw, source, score)| {
                        (
                            serde_json::from_str::<Message>(&raw).unwrap().id,
                            source,
                            score,
                        )
                    })
                    .collect();
                let expected: Vec<_> = full
                    .into_iter()
                    .map(|(m, source, score)| (m.id, source, score))
                    .collect();
                assert_eq!(actual, expected, "{raw}");
            }
        }
        for (raw, words, variants, proven) in [
            (
                "historical correspondence",
                vec!["historical", "correspondence"],
                vec![vec!["historical"], vec!["correspondence"]],
                true,
            ),
            (
                "historical correspondence garden",
                vec!["historical", "correspondence", "garden"],
                vec![vec!["historical"], vec!["correspondence"], vec!["garden"]],
                true,
            ),
            (
                "garden historical",
                vec!["garden", "historical"],
                vec![vec!["garden"], vec!["historical"]],
                true,
            ),
            (
                "historial correspondence",
                vec!["historial", "correspondence"],
                vec![vec!["historial", "historical"], vec!["correspondence"]],
                true,
            ),
            (
                "historical correspondence care",
                vec!["historical", "correspondence", "care"],
                vec![vec!["historical"], vec!["correspondence"], vec!["care"]],
                true,
            ),
            (
                "local extracted attachment",
                vec!["local", "extracted", "attachment"],
                vec![vec!["local"], vec!["extracted"], vec!["attachment"]],
                true,
            ),
            (
                "garden care",
                vec!["garden", "care"],
                vec![vec!["garden"], vec!["care"]],
                false,
            ),
            (
                "maya historical",
                vec!["maya", "historical"],
                vec![vec!["maya"], vec!["historical"]],
                false,
            ),
            (
                "raretoken garden",
                vec!["raretoken", "garden"],
                vec![vec!["raretoken"], vec!["garden"]],
                false,
            ),
            (
                "absent garden",
                vec!["absent", "garden"],
                vec![vec!["absent"], vec!["garden"]],
                false,
            ),
        ] {
            let units: Vec<_> = words.iter().map(|unit| unit.to_string()).collect();
            let expanded: Vec<Vec<String>> = variants
                .iter()
                .map(|variants| variants.iter().map(|variant| variant.to_string()).collect())
                .collect();
            let canonical: Vec<Vec<String>> = variants
                .iter()
                .map(|variants| {
                    variants
                        .iter()
                        .map(|variant| crate::lexical_text::unit(variant).unwrap().unwrap())
                        .collect()
                })
                .collect();
            let phrase = crate::lexical_text::unit(raw).unwrap().unwrap();
            for (label, seen, starred, from) in [
                ("all", None, None, None),
                ("unread", Some(false), None, None),
                ("sparse", Some(true), Some(true), None),
                ("sender", None, None, Some("nora@example.invalid")),
                ("absent sender", None, None, Some("absent@example.invalid")),
            ] {
                let at = crate::now();
                let filter = CachedSearchFilter {
                    text: Some(raw),
                    text_units: Some(&units),
                    text_alternatives: Some(&expanded),
                    seen,
                    starred,
                    from,
                    ..Default::default()
                };
                let bounded = ranked_head(
                    &target,
                    &account,
                    &canonical,
                    raw,
                    Some(&phrase),
                    7,
                    Some((&filter, &at)),
                )
                .await
                .unwrap();
                if label == "all" {
                    assert_eq!(bounded.is_some(), proven, "{raw}");
                }
                let full = target
                    .lexical_rows(
                        &account,
                        &filter,
                        LexicalPage {
                            ranked: true,
                            limit: 7,
                            verify_plan: true,
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                let expected: Vec<_> = full
                    .into_iter()
                    .map(|(m, s, score)| (m.id, s, score))
                    .collect();
                if let Some(bounded) = bounded {
                    let actual: Vec<_> = bounded
                        .into_iter()
                        .map(|(m, s, score)| {
                            (serde_json::from_str::<Message>(&m).unwrap().id, s, score)
                        })
                        .collect();
                    assert_eq!(actual, expected, "{raw}/{label}");
                }
                let automatic = target
                    .lexical_rows(
                        &account,
                        &filter,
                        LexicalPage {
                            ranked: true,
                            limit: 7,
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    automatic
                        .into_iter()
                        .map(|(m, s, score)| (m.id, s, score))
                        .collect::<Vec<_>>(),
                    expected,
                    "automatic {raw}/{label}"
                );
            }
        }
        let work = vec!["Work".to_owned()];
        let missing = vec!["missing".to_owned()];
        let units = vec!["garden".to_owned()];
        let canonical = vec!["wgarden".to_owned()];
        let at = crate::now();
        for (label, mut filter, expected) in [
            (
                "unread",
                CachedSearchFilter {
                    seen: Some(false),
                    ..Default::default()
                },
                true,
            ),
            (
                "read",
                CachedSearchFilter {
                    seen: Some(true),
                    ..Default::default()
                },
                true,
            ),
            (
                "starred",
                CachedSearchFilter {
                    starred: Some(true),
                    ..Default::default()
                },
                true,
            ),
            (
                "sparse intersection",
                CachedSearchFilter {
                    seen: Some(false),
                    starred: Some(true),
                    ..Default::default()
                },
                false,
            ),
            (
                "sender",
                CachedSearchFilter {
                    from: Some("Nora@Example.invalid"),
                    ..Default::default()
                },
                true,
            ),
            (
                "missing sender",
                CachedSearchFilter {
                    from: Some("absent@example.invalid"),
                    ..Default::default()
                },
                false,
            ),
            (
                "recipient",
                CachedSearchFilter {
                    to: Some("carbon@example.invalid"),
                    ..Default::default()
                },
                true,
            ),
            (
                "subject",
                CachedSearchFilter {
                    subject: Some("CARE"),
                    ..Default::default()
                },
                true,
            ),
            (
                "lower score stratum",
                CachedSearchFilter {
                    subject: Some("Other"),
                    ..Default::default()
                },
                false,
            ),
            (
                "before",
                CachedSearchFilter {
                    before: Some("2021-01-01"),
                    ..Default::default()
                },
                true,
            ),
            (
                "since",
                CachedSearchFilter {
                    since: Some("2026-01-01"),
                    ..Default::default()
                },
                false,
            ),
            (
                "date interval",
                CachedSearchFilter {
                    since: Some("2020-01-01"),
                    before: Some("2021-01-01"),
                    ..Default::default()
                },
                true,
            ),
            (
                "folder",
                CachedSearchFilter {
                    folders: Some(&work),
                    ..Default::default()
                },
                true,
            ),
            (
                "missing folder",
                CachedSearchFilter {
                    folders: Some(&missing),
                    ..Default::default()
                },
                false,
            ),
            (
                "category",
                CachedSearchFilter {
                    category: Some("people"),
                    ..Default::default()
                },
                true,
            ),
            (
                "snoozed",
                CachedSearchFilter {
                    snoozed_only: true,
                    ..Default::default()
                },
                true,
            ),
            (
                "awake",
                CachedSearchFilter {
                    hide_snoozed: true,
                    ..Default::default()
                },
                true,
            ),
            (
                "contradiction",
                CachedSearchFilter {
                    snoozed_only: true,
                    hide_snoozed: true,
                    ..Default::default()
                },
                false,
            ),
        ] {
            filter.text = Some("garden");
            filter.text_units = Some(&units);
            let bounded = single_unit_head(
                &target,
                &account,
                &canonical,
                "garden",
                false,
                7,
                Some((&filter, &at)),
            )
            .await
            .unwrap();
            assert_eq!(bounded.is_some(), expected, "{label}");
            let full = target
                .lexical_rows(
                    &account,
                    &filter,
                    LexicalPage {
                        ranked: true,
                        limit: 7,
                        verify_plan: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let expected: Vec<_> = full
                .into_iter()
                .map(|(m, s, score)| (m.id, s, score))
                .collect();
            if let Some(bounded) = bounded {
                let actual: Vec<_> = bounded
                    .into_iter()
                    .map(|(m, s, score)| {
                        (serde_json::from_str::<Message>(&m).unwrap().id, s, score)
                    })
                    .collect();
                assert_eq!(actual, expected, "{label}");
            }
            let optimized = target
                .lexical_rows(
                    &account,
                    &filter,
                    LexicalPage {
                        ranked: true,
                        limit: 7,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let actual: Vec<_> = optimized
                .into_iter()
                .map(|(m, s, score)| (m.id, s, score))
                .collect();
            assert_eq!(actual, expected, "automatic planning: {label}");
            if label == "sparse intersection" {
                assert_eq!(actual.len(), 7);
                assert!(
                    actual.iter().any(|(_, _, score)| *score == -1.),
                    "lower-scoring eligible rows must fill the page"
                );
            }
        }
        if let SearchDatabase::Sqlite(pool) = &target {
            let filter = CachedSearchFilter {
                from: Some("nora@example.invalid"),
                text: Some("garden"),
                text_units: Some(&units),
                ..Default::default()
            };
            let full = target
                .lexical_rows(
                    &account,
                    &filter,
                    LexicalPage {
                        ranked: true,
                        limit: 7,
                        verify_plan: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            sqlx::query("INSERT INTO metadata_terms(metadata_terms) VALUES('delete-all')")
                .execute(pool)
                .await
                .unwrap();
            assert!(single_unit_head(
                &target,
                &account,
                &canonical,
                "garden",
                false,
                7,
                Some((&filter, &at))
            )
            .await
            .unwrap()
            .is_some());
            let actual = target
                .lexical_rows(
                    &account,
                    &filter,
                    LexicalPage {
                        ranked: true,
                        limit: 7,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                actual.len(),
                7,
                "missing secondary postings must preserve canonical metadata results"
            );
            assert_eq!(
                actual
                    .into_iter()
                    .map(|(m, s, score)| (m.id, s, score))
                    .collect::<Vec<_>>(),
                full.into_iter()
                    .map(|(m, s, score)| (m.id, s, score))
                    .collect::<Vec<_>>()
            );
            sqlx::query("INSERT INTO metadata_terms(metadata_terms) VALUES('rebuild')")
                .execute(pool)
                .await
                .unwrap();
            assert!(single_unit_head(
                &target,
                &account,
                &canonical,
                "garden",
                false,
                7,
                Some((&filter, &at))
            )
            .await
            .unwrap()
            .is_some());
        }
        // The proof is account scoped, and survives reopening persisted state.
        assert!(single_unit_head(
            &target,
            "other-owner",
            &["wgarden".into()],
            "garden",
            false,
            51,
            None
        )
        .await
        .unwrap()
        .is_none());
        let reopened = SearchDatabase::open(dir.path(), url).await.unwrap();
        assert!(single_unit_head(
            &reopened,
            &account,
            &["wgarden".into()],
            "garden",
            false,
            51,
            None
        )
        .await
        .unwrap()
        .is_some());
        // A complete page outside the proof prefix must still be discoverable.
        let mut newer = vec![];
        for n in 0..1025 {
            let id = format!("newer-{n:04}");
            let original = records[1].2.as_ref().unwrap();
            let mut document = Document {
                message: original.message.clone(),
                source: id.clone(),
                sender: original.sender.clone(),
                recipients: original.recipients.clone(),
                mime_recipients: String::new(),
                attachment_details: "[]".into(),
                body_payload: true,
                snoozed_until: None,
                subject: original.subject.clone(),
                body: "unrelated correspondence".into(),
                participants: String::new(),
                attachments: String::new(),
                state: "ready".into(),
            };
            document.message.id = id.clone();
            document.message.received_at = "2090-01-01T00:00:00Z".into();
            newer.push((id, 1, Some(document)));
        }
        target.deliver_batch(&account, &newer).await.unwrap();
        while target.lexical_turn(&account, true).await.unwrap() {}
        assert!(
            single_unit_head(&target, &account, &canonical, "garden", false, 7, None)
                .await
                .unwrap()
                .is_none()
        );
        let filter = CachedSearchFilter {
            text: Some("garden"),
            text_units: Some(&units),
            ..Default::default()
        };
        let mut pages = vec![];
        for verify_plan in [false, true] {
            pages.push(
                target
                    .lexical_rows(
                        &account,
                        &filter,
                        LexicalPage {
                            ranked: true,
                            limit: 7,
                            verify_plan,
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|(m, s, score)| (m.id, s, score))
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(pages[0].len(), 7);
        assert_eq!(pages[0], pages[1], "prefix must never cap full ranking");
        // Tied dates span the boundary; the ordered proof must still match the complete head.
        let dense = single_unit_head(
            &target,
            &account,
            &["wunrelated".into()],
            "unrelated",
            false,
            7,
            None,
        )
        .await
        .unwrap()
        .expect("dense prefix proves the head");
        let dense_units = vec!["unrelated".to_owned()];
        let full = target
            .lexical_rows(
                &account,
                &CachedSearchFilter {
                    text: Some("unrelated"),
                    text_units: Some(&dense_units),
                    ..Default::default()
                },
                LexicalPage {
                    ranked: true,
                    limit: 7,
                    verify_plan: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            dense
                .into_iter()
                .map(|(raw, source, score)| (
                    serde_json::from_str::<Message>(&raw).unwrap().id,
                    source,
                    score
                ))
                .collect::<Vec<_>>(),
            full.into_iter()
                .map(|(message, source, score)| (message.id, source, score))
                .collect::<Vec<_>>()
        );
        target.remove_account(&account).await.unwrap();
    }
    #[tokio::test]
    async fn sqlite_score_ceiling_heads_match_complete_ranking() {
        contract(None).await;
    }
    #[tokio::test]
    #[ignore = "requires isolated pgvector PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
    async fn postgres_score_ceiling_heads_match_complete_ranking() {
        contract(Some(&std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap())).await;
    }
}

#[cfg(test)]
#[path = "search_bound_head_diagnostic.rs"]
mod diagnostic;
