//! The single writer (spec 03 §1, 00 runtime shape): one thread owns the
//! read-write connection and runs queued writes in batched transactions.
//! Each write gets its own savepoint, so one failing write leaves the rest
//! of the batch intact.

use std::path::Path;
use std::sync::mpsc;
use std::thread::JoinHandle;

use rusqlite::Connection;
use tokio::sync::oneshot;

use crate::{Error, PRAGMAS};

/// Runs one write and returns how to answer its caller once the batch's
/// commit is known: a write is reported only after it is durable.
type Job = Box<dyn FnOnce(&mut Connection) -> Answer + Send>;
type Answer = Box<dyn FnOnce(Option<&str>) + Send>;

/// Writes at most this many queued jobs per transaction.
const BATCH: usize = 64;

/// A handle on the writer thread. Dropping it stops the thread once the
/// queue is empty.
#[derive(Debug)]
pub struct Writer {
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

/// The result of one write; await it, or `wait` outside async code.
#[derive(Debug)]
pub struct PendingWrite<R>(oneshot::Receiver<Result<R, Error>>);

impl<R> PendingWrite<R> {
    /// Blocks until the write has committed or failed.
    pub fn wait(self) -> Result<R, Error> {
        self.0.blocking_recv().unwrap_or(Err(Error::WriterStopped))
    }
}

impl<R> std::future::Future for PendingWrite<R> {
    type Output = Result<R, Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0)
            .poll(cx)
            .map(|r| r.unwrap_or(Err(Error::WriterStopped)))
    }
}

impl Writer {
    pub fn start(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(PRAGMAS)?;
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("mado-sqlite-writer".into())
            .spawn(move || run(conn, &rx))?;
        Ok(Self {
            jobs: Some(tx),
            thread: Some(thread),
        })
    }

    /// Queues `f` to run inside the next batch's transaction. `f` sees the
    /// connection inside its own savepoint: an error rolls back only `f`.
    pub fn write<R, F>(&self, f: F) -> PendingWrite<R>
    where
        R: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
    {
        let (done, result) = oneshot::channel();
        let job: Job = Box::new(move |conn: &mut Connection| {
            let outcome = (|| -> rusqlite::Result<R> {
                let sp = conn.savepoint()?;
                let value = f(&sp)?;
                sp.commit()?;
                Ok(value)
            })();
            Box::new(move |commit_error: Option<&str>| {
                let result = match (outcome, commit_error) {
                    (Ok(_), Some(e)) => Err(Error::CommitFailed(e.to_owned())),
                    (outcome, _) => outcome.map_err(Error::from),
                };
                let _ = done.send(result);
            })
        });
        if let Some(jobs) = &self.jobs {
            // A send fails only when the thread is gone; the dropped sender
            // half then answers WriterStopped.
            let _ = jobs.send(job);
        }
        PendingWrite(result)
    }
}

fn run(mut conn: Connection, rx: &mpsc::Receiver<Job>) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(job) => batch.push(job),
                Err(_) => break,
            }
        }
        // The batch shares one transaction; BEGIN IMMEDIATE takes the write
        // lock up front, so a busy app database waits here (busy_timeout),
        // not halfway through.
        if conn.execute_batch("BEGIN IMMEDIATE").is_err() {
            // No batch transaction: each write's savepoint is then its own
            // transaction, committed when the savepoint is.
            for job in batch {
                job(&mut conn)(None);
            }
            continue;
        }
        let answers: Vec<Answer> = batch.into_iter().map(|job| job(&mut conn)).collect();
        let commit = conn.execute_batch("COMMIT").map_err(|e| {
            let _ = conn.execute_batch("ROLLBACK");
            e.to_string()
        });
        let failure = commit.err();
        for answer in answers {
            answer(failure.as_deref());
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        drop(self.jobs.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
