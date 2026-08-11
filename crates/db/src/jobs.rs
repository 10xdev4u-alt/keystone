//! Background jobs with exactly-one-runner semantics.
//!
//! [`run_exclusive`] takes a session-level Postgres advisory lock keyed by the
//! job name, runs the work while holding it, and releases it afterwards. Any
//! number of API nodes can race the same job: exactly one acquires the lock,
//! the rest return `false` immediately. Locks are session-scoped (not
//! transaction-scoped), so long-running work holds the lock for its whole
//! duration — the property that actually guarantees a single runner.
//!
//! Jobs are plain async functions; the registry in the API layer maps names
//! to work (stats aggregation, digests, session cleanup, ...).

use sqlx::PgPool;
use std::future::Future;

/// Run `work` under the job's advisory lock. Returns `true` when THIS caller
/// held the lock and ran the work, `false` when another runner is active.
/// Locks auto-release on connection close (crash-safe).
pub async fn run_exclusive<F, T>(pool: &PgPool, job: &str, work: F) -> Result<bool, sqlx::Error>
where
    F: Future<Output = T>,
{
    // The lock key is a stable 64-bit hash of the job name.
    let key: i64 = stable_job_key(job);

    // CRITICAL: hold a dedicated connection for the whole run. Advisory locks
    // are session-scoped and re-entrant per session — if the connection were
    // returned to the pool after the acquire query, every later runner handed
    // that same session would also "acquire" the lock.
    let mut conn = pool.acquire().await?;

    // `pg_try_advisory_lock` fails fast instead of queueing — the whole point
    // of a distributed mutex for cron-like jobs.
    let acquired = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
        .bind(key)
        .fetch_one(&mut *conn)
        .await?;
    if !acquired {
        return Ok(false);
    }

    work.await;
    // Release is best-effort: if the connection died mid-job the lock already
    // disappeared with it (the session-scoped lock dies with the session).
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(key)
        .execute(&mut *conn)
        .await;
    Ok(true)
}

/// A stable, positive i64 key per job name (pg advisory locks take bigint).
fn stable_job_key(job: &str) -> i64 {
    // FNV-1a 64-bit, then mask the sign bit for a positive key.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in job.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash & i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_keys_are_stable_and_positive() {
        let a = stable_job_key("stats");
        let b = stable_job_key("stats");
        let c = stable_job_key("digest");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a > 0 && c > 0);
    }
}
