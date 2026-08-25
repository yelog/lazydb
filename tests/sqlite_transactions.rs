use sqlx::{Database, Executor, Sqlite, sqlite::SqlitePoolOptions};
use sqlx_core::transaction::TransactionManager;

#[tokio::test]
async fn pinned_sqlite_connection_commits_and_rolls_back() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let mut connection = pool.acquire().await.unwrap();
    connection
        .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
        .await
        .unwrap();

    <Sqlite as Database>::TransactionManager::begin(&mut connection, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO items (value) VALUES ('rollback')")
        .execute(&mut *connection)
        .await
        .unwrap();
    <Sqlite as Database>::TransactionManager::rollback(&mut connection)
        .await
        .unwrap();
    assert_eq!(
        <Sqlite as Database>::TransactionManager::get_transaction_depth(&connection),
        0
    );

    <Sqlite as Database>::TransactionManager::begin(&mut connection, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO items (value) VALUES ('commit')")
        .execute(&mut *connection)
        .await
        .unwrap();
    <Sqlite as Database>::TransactionManager::commit(&mut connection)
        .await
        .unwrap();
    assert_eq!(
        <Sqlite as Database>::TransactionManager::get_transaction_depth(&connection),
        0
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM items")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
