extern crate postgres;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate uuid;
extern crate rand;
extern crate rayon;

use postgres::{Connection, Result};
use postgres::transaction::Transaction;
use postgres::error::T_R_SERIALIZATION_FAILURE;
use r2d2_postgres::{TlsMode, PostgresConnectionManager};
use uuid::Uuid;
use rand::{Rng, Rand, XorShiftRng, thread_rng};
use rand::distributions::{IndependentSample, Range};
use rayon::prelude::*;

use std::time::SystemTime;
use std::cmp::max;

const COCKROACHDB_URL: &'static str = "postgresql://testuser@localhost:26257/testdb";
const POSTGRESQL_URL: &'static str = "postgresql://postgres:testpw@localhost:5432/testdb";

fn execute_txn<T, F>(conn: &Connection, mut op: F) -> Result<T>
where
    F: FnMut(&Transaction) -> Result<T>,
{
    let txn = conn.transaction()?;
    loop {
        let sp = txn.savepoint("cockroach_restart")?;
        match op(&sp).and_then(|t| sp.commit().map(|_| t)) {
            Err(ref err) if err.as_db()
                               .map(|e| e.code == T_R_SERIALIZATION_FAILURE)
                               .unwrap_or(false) => {},
            r => break r,
        }
    }.and_then(|t| txn.commit().map(|_| t))
}

fn insert_user(txn: &Transaction, user_id: Uuid, database: &str) -> Result<()> {
    let mut rng1 = XorShiftRng::new_unseeded();
    let mut rng2 = XorShiftRng::new_unseeded();
    let num_docs_range: Range<i64> = Range::new(10, 1000);
    let num_revisions_range: Range<i64> = Range::new(1, 20);

    let num_docs = num_docs_range.ind_sample(&mut rng1);
    for _ in 0..num_docs {
        let doc_id = Uuid::rand(&mut rng1);
        let num_revisions = num_revisions_range.ind_sample(&mut rng2);
        for revision in 0..num_revisions {
            let payload = rng2.gen_iter::<u8>().take(2048).collect::<Vec<_>>();
            let query = match database {
                "cockroachdb" => "INSERT INTO docs (user_id, doc_id, revision, payload) VALUES ($1, $2, $3, $4) RETURNING NOTHING",
                "postgresql" => "INSERT INTO docs (user_id, doc_id, revision, payload) VALUES ($1, $2, $3, $4)",
                _ => panic!("invalid database")
            };
            txn.execute(query, &[&user_id, &doc_id, &revision, &payload])?;
        }
    }
    Ok(())
}

fn select_docs(conn: &Connection, user_id: Uuid, batch_size: usize, iterations: usize) -> Result<()> {
    let mut rng1 = XorShiftRng::new_unseeded();
    let mut rng2 = thread_rng();
    let num_docs_range: Range<usize> = Range::new(10, 1000);

    let num_docs = num_docs_range.ind_sample(&mut rng1);
    let all_doc_ids = (0..num_docs).into_iter().map(|_| Uuid::rand(&mut rng1)).collect::<Vec<_>>();

    let index_range: Range<usize> = Range::new(0, num_docs);

    for _ in 0..iterations {
        if batch_size == 0 {
            let index = index_range.ind_sample(&mut rng2);
            let doc_id = all_doc_ids[index];

            let query = "
                SELECT
                    docs.user_id, docs.doc_id, docs.revision, docs.payload
                FROM
                    docs
                WHERE
                    docs.user_id = $1
                AND
                    docs.doc_id = $2
                ORDER BY
                    docs.revision DESC
                LIMIT 1
            ";

            conn.execute(&query, &[&user_id, &doc_id])?;
        } else {
            let doc_ids = (0..batch_size)
                .into_iter()
                .map(|_| {
                    let index = index_range.ind_sample(&mut rng2);
                    all_doc_ids[index]
                })
                .collect::<Vec<_>>();

            let query = "
                SELECT DISTINCT ON(user_id, doc_id)
                    user_id, doc_id, revision, payload
                FROM docs
                WHERE docs.user_id = $1 AND docs.doc_id = ANY ($2)
                ORDER BY user_id, doc_id, revision DESC
            ";

            conn.execute(&query, &[&user_id, &doc_ids])?;
        }
    }
    Ok(())
}

fn main() {
    let action = std::env::args().nth(1).expect("action argument missing: 'load' or 'run'");
    let database = std::env::args().nth(2).expect("database argument missing: 'cockroachdb' or 'postgresql'");
    let batch_size = if action == "run" {
        std::env::args().nth(3).and_then(|arg| arg.parse::<usize>().ok()).expect("batch size argument missing: 0 = single, 1-n = batch")
    } else {
        0
    };

    let url = match database.as_str() {
        "cockroachdb" => COCKROACHDB_URL,
        "postgresql" => POSTGRESQL_URL,
        _ => panic!("invalid database")
    };
    let manager = PostgresConnectionManager::new(url, TlsMode::None).unwrap();
    let pool = r2d2::Pool::builder()
        .max_size(64)
        .build(manager)
        .unwrap();
    let mut rng = XorShiftRng::new_unseeded();

    let user_count = 100;
    let user_ids = (0..user_count).into_iter().map(|_| Uuid::rand(&mut rng)).collect::<Vec<_>>();

    let now = SystemTime::now();
    let mut iterations = 1;

    match action.as_str() {
        "load" => {
            user_ids
                .into_par_iter()
                .map(|user_id| {
                    let local_pool = pool.clone();
                    let conn = local_pool.get().unwrap();
                    execute_txn(&conn, |txn| insert_user(txn, user_id, &database)).unwrap();
                })
                .count();
        }
        "run" => {
            iterations = 100;
            user_ids
                .into_par_iter()
                .map(|user_id| {
                    let local_pool = pool.clone();
                    let conn = local_pool.get().unwrap();
                    select_docs(&conn, user_id, batch_size, iterations).unwrap();
                })
                .count();
        }
        _ => panic!("invalid action")
    }

    let duration = (now.elapsed().unwrap().as_secs() as f64) * 1_000_000_000f64 + now.elapsed().unwrap().subsec_nanos() as f64;

    let transactions_per_second = user_count as f64 * iterations as f64 * 1_000_000_000f64 / duration;
    println!("transactions/s: {}", transactions_per_second);

    if action == "run" {
        let rows_per_transaction = max(batch_size, 1);
        println!("rows/s: {}", rows_per_transaction as f64 * transactions_per_second);
    }
}
