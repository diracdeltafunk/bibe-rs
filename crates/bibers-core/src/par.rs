//! Data parallelism for the per-entry loops of the pipeline (DC-068).
//!
//! The pipeline is a port of biber's serial loops, and its output (the
//! `.bbl`, the log, warnings and their order) must not depend on the number
//! of threads. Parallel loops therefore only *compute* per-item results; the
//! caller applies them, and their side effects (state changes, log messages
//! from a forked [`Logger`](crate::log::Logger)), in the serial order:
//!
//! - [`map`] maps a function over a slice and returns the results in order;
//! - [`map_chunks`] does the same with per-chunk state (a cache, a forked
//!   logger, recorded side effects) that the caller merges chunk by chunk.
//!
//! Both run serially with one thread (`--threads 1`) and for slices shorter
//! than the threshold (thread start-up and splitting would cost more than
//! they save on a small input).
//!
//! Settings: `--threads N` (bibers' command line) or `$BIBERS_THREADS`; the
//! default is the number of available cores. `$BIBERS_PAR_MIN_ITEMS` lowers
//! or raises the threshold (tests set it to 1 so that the corpus's small
//! inputs exercise the parallel paths).

use crate::log::{Fatal, Logger};
use rayon::prelude::*;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The default minimum slice length for a parallel loop.
const DEFAULT_MIN_ITEMS: usize = 64;

/// 0 = not set yet (resolved from the environment on first use).
static THREADS: AtomicUsize = AtomicUsize::new(0);
static MIN_ITEMS: OnceLock<usize> = OnceLock::new();
static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();

/// Set the number of threads (`--threads`; 0 = the default). Call before the
/// first parallel loop: the thread pool is built once, on first use.
pub fn set_threads(n: usize) {
    THREADS.store(
        if n == 0 { default_threads() } else { n },
        Ordering::Relaxed,
    );
}

fn default_threads() -> usize {
    let env = std::env::var("BIBERS_THREADS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0);
    env.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from))
}

/// The number of threads parallel loops use.
pub fn threads() -> usize {
    match THREADS.load(Ordering::Relaxed) {
        0 => {
            let n = default_threads();
            THREADS.store(n, Ordering::Relaxed);
            n
        }
        n => n,
    }
}

fn min_items() -> usize {
    *MIN_ITEMS.get_or_init(|| {
        std::env::var("BIBERS_PAR_MIN_ITEMS")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_MIN_ITEMS)
            .max(1)
    })
}

/// The thread pool, if loops over `len` items should run in parallel.
fn pool_for(len: usize) -> Option<&'static rayon::ThreadPool> {
    let n = threads();
    if n <= 1 || len < min_items().max(2) {
        return None;
    }
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("bibers-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Whether a loop over `len` items would run in parallel.
pub fn is_parallel(len: usize) -> bool {
    pool_for(len).is_some()
}

/// `items.iter().map(f).collect()`, in parallel when enabled. The results
/// are in the order of `items`.
pub fn map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync + Send,
{
    match pool_for(items.len()) {
        None => items.iter().map(f).collect(),
        Some(pool) => pool.install(|| items.par_iter().map(f).collect()),
    }
}

/// The chunk length for `len` items: a few chunks per thread, so that
/// uneven items still balance.
fn chunk_len(len: usize) -> usize {
    len.div_ceil(threads() * 4).max(1)
}

/// Map `f` over `items` in contiguous chunks, each with its own state from
/// `init`. Returns, in order, each chunk's final state and results. Serially
/// this is one chunk: `init()` once, then `f` over every item.
///
/// `f` returning `Err` stops its chunk (the remaining items of the chunk get
/// no result), like `?` in a serial loop; the caller merges the chunks in
/// order and stops at the first error, so later chunks' work is discarded.
pub fn map_chunks<T, S, R, E, I, F>(items: &[T], init: I, f: F) -> Vec<Chunk<S, R, E>>
where
    T: Sync,
    S: Send,
    R: Send,
    E: Send,
    I: Fn() -> S + Sync + Send,
    F: Fn(&mut S, &T) -> Result<R, E> + Sync + Send,
{
    let run = |chunk: &[T]| {
        let mut state = init();
        let mut results = Vec::with_capacity(chunk.len());
        let mut error = None;
        for item in chunk {
            match f(&mut state, item) {
                Ok(r) => results.push(r),
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        Chunk {
            state,
            results,
            error,
        }
    };
    match pool_for(items.len()) {
        None => vec![run(items)],
        Some(pool) => {
            let len = chunk_len(items.len());
            pool.install(|| items.par_chunks(len).map(run).collect())
        }
    }
}

/// [`map`] for a fallible per-item computation that logs: each chunk logs to
/// a fork of `log`, and the forks are joined in order. Returns the results
/// up to the first error and that error, exactly as a serial loop with `?`
/// would have logged them (the caller applies the results, then returns the
/// error).
pub fn map_logged<T, R, F>(log: &mut Logger, items: &[T], f: F) -> (Vec<R>, Result<(), Fatal>)
where
    T: Sync,
    R: Send,
    F: Fn(&mut Logger, &T) -> Result<R, Fatal> + Sync + Send,
{
    let parent: &Logger = log;
    let chunks = map_chunks(items, || parent.fork(), &f);
    let mut results = Vec::with_capacity(items.len());
    for c in chunks {
        log.join(c.state);
        results.extend(c.results);
        if let Some(e) = c.error {
            return (results, Err(e));
        }
    }
    (results, Ok(()))
}

/// `items` split into the contiguous chunks [`map_chunks`] would use (one
/// chunk when the loop would run serially), for a [`map`] over chunks whose
/// per-chunk state need not be `Send`.
pub fn split<T>(items: &[T]) -> Vec<&[T]> {
    if pool_for(items.len()).is_none() {
        return vec![items];
    }
    items.chunks(chunk_len(items.len())).collect()
}

/// One chunk's output from [`map_chunks`].
pub struct Chunk<S, R, E> {
    /// The chunk's state after its last item.
    pub state: S,
    /// One result per item, up to the first error.
    pub results: Vec<R>,
    /// The error that stopped the chunk.
    pub error: Option<E>,
}

/// Run `a` and `b`, in parallel when enabled; returns both results.
pub fn join<A, B, RA, RB>(a: A, b: B) -> (RA, RB)
where
    A: FnOnce() -> RA + Send,
    B: FnOnce() -> RB + Send,
    RA: Send,
    RB: Send,
{
    match pool_for(usize::MAX) {
        None => (a(), b()),
        Some(pool) => pool.join(a, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_keeps_order() {
        let v: Vec<u32> = (0..10_000).collect();
        let r = map(&v, |x| x * 2);
        assert_eq!(r, v.iter().map(|x| x * 2).collect::<Vec<_>>());
    }

    #[test]
    fn chunks_stop_at_errors() {
        let v: Vec<u32> = (0..1000).collect();
        let out = map_chunks(&v, Vec::new, |seen: &mut Vec<u32>, &x| {
            seen.push(x);
            if x == 500 { Err(x) } else { Ok(x) }
        });
        let mut all = Vec::new();
        for c in out {
            all.extend(c.results);
            if let Some(e) = c.error {
                assert_eq!(e, 500);
                break;
            }
        }
        assert_eq!(all, (0..500).collect::<Vec<_>>());
    }
}
