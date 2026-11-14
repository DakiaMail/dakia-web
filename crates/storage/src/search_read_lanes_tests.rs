use super::*;
async fn contract(url: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let connection = SearchConnection::new(dir.path(), url).unwrap();
    connection.warm().await.unwrap();
    macro_rules! verify {
        ($ranking:expr,$semantic:expr) => {{
            let mut held = Vec::new();
            for _ in 0..$ranking.options().get_max_connections() {
                held.push($ranking.acquire().await.unwrap());
            }
            assert!(tokio::time::timeout(
                std::time::Duration::from_millis(20),
                connection.ranked_candidates("missing", &CachedSearchFilter::default(), None, 51)
            )
            .await
            .is_err());
            // Every ranking slot remains occupied. Other components must have
            // independent physical capacity, including the control repository.
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                let ids = vec!["missing".into()];
                let constraints = vec!["strict phrase".into()];
                let filter = CachedSearchFilter::default();
                let (coverage, semantic, phrases, metadata) = tokio::join!(
                    connection.coverage("missing"),
                    sqlx::query_scalar::<_, i64>("SELECT CAST(1 AS BIGINT)").fetch_one(&$semantic),
                    connection.phrase_ids("missing", &ids, &constraints),
                    connection.rows("missing", &filter, None)
                );
                assert_eq!(coverage.unwrap().headers, 0);
                assert_eq!(semantic.unwrap(), 1);
                assert!(phrases.unwrap().is_empty());
                assert!(metadata.unwrap().is_empty());
            })
            .await
            .unwrap();
            drop(held);
        }};
    }
    match (
        connection.lexical_connection().await.unwrap(),
        connection.semantic_connection().await.unwrap(),
    ) {
        (SearchDatabase::Sqlite(ranking), SearchDatabase::Sqlite(semantic)) => {
            verify!(ranking, semantic)
        }
        (SearchDatabase::Postgres(ranking), SearchDatabase::Postgres(semantic)) => {
            for pool in [&ranking, &semantic] {
                assert_eq!(
                    sqlx::query_scalar::<_, String>("SHOW jit")
                        .fetch_one(pool)
                        .await
                        .unwrap(),
                    "off"
                );
            }
            verify!(ranking, semantic)
        }
        _ => panic!("read lanes must use the configured backend"),
    }
}
#[tokio::test]
async fn sqlite_ranking_saturation_preserves_semantic_phrase_and_control_capacity() {
    contract(None).await;
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
async fn postgres_ranking_saturation_preserves_semantic_phrase_and_control_capacity() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap();
    assert!(url.split('?').next().unwrap().ends_with("_test"));
    contract(Some(&url)).await;
}
