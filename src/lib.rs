//! A crate that wraps some functionality from [tokio-timer] for use in futures 0.3 code that
//! doesn't use tokio.
//!
//! [tokio-timer]: https://crates.io/crates/tokio-timer

#![warn(
    rust_2018_idioms,
    deprecated_in_future,
    macro_use_extern_crate,
    missing_debug_implementations,
    unused_labels,
    unused_qualifications,
    clippy::cast_possible_truncation
)]

pub use tokio_timer::{timeout::Error as TimeoutError, Error};

use crossbeam_channel::{Sender, TryRecvError};
use futures::{
    compat::{Compat, Future01CompatExt},
    prelude::*,
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tokio_executor::park::ParkThread;
use tokio_timer::{timer::Handle, Timer};

/// Provider of all timer functionality
///
/// This type can be cheaply cloned; you should generally only construct a single one by calling
/// [`TimerProvider::new()`], and then clone that one if you need more handles.
#[derive(Debug, Clone)]
pub struct TimerProvider(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    timer_handle: Handle,
    shutdown_tx: Sender<()>,
}

impl TimerProvider {
    /// Creates a `TimerProvider`, spawning a background thread to drive timer events.
    pub fn new() -> Self {
        let (timer_tx, timer_rx) = crossbeam_channel::bounded(0);
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
        thread::spawn(move || {
            let mut timer = Timer::new(ParkThread::new());
            timer_tx.send(timer.handle()).unwrap();

            loop {
                timer.turn(Some(Duration::from_millis(50))).unwrap();

                match shutdown_rx.try_recv() {
                    Err(TryRecvError::Empty) => {}
                    Ok(()) | Err(TryRecvError::Disconnected) => break,
                }
            }
        });

        TimerProvider(Arc::new(Inner {
            timer_handle: timer_rx.recv().unwrap(),
            shutdown_tx,
        }))
    }

    /// Creates a future that completes at the specified instant in time.
    pub fn delay(&self, deadline: Instant) -> impl Future<Output = Result<(), Error>> + Unpin {
        self.0.timer_handle.delay(deadline).compat()
    }

    /// Creates a future that completes after `duration` amount of time, starting from the current
    /// instant.
    pub fn delay_from_now(
        &self,
        duration: Duration,
    ) -> impl Future<Output = Result<(), Error>> + Unpin {
        self.delay(Instant::now() + duration)
    }

    /// Wraps a future, setting an upper bound to the amount of time it is allowed to execute. If
    /// the future or stream does not complete by `deadline`, it is canceled and an error is
    /// returned.
    pub fn timeout_future<T, E, Fut>(
        &self,
        fut: Fut,
        deadline: Instant,
    ) -> impl Future<Output = Result<T, TimeoutError<E>>> + Unpin
    where
        Fut: Future<Output = Result<T, E>> + Unpin,
    {
        self.0
            .timer_handle
            .timeout(Compat::new(fut), deadline)
            .compat()
    }

    /// Wraps a future, setting an upper bound to the amount of time it is allowed to execute. If
    /// the future or stream does not complete after `duration` amount of time starting from now, it
    /// is canceled and an error is returned.
    pub fn timeout_future_from_now<T, E, Fut>(
        &self,
        fut: Fut,
        duration: Duration,
    ) -> impl Future<Output = Result<T, TimeoutError<E>>> + Unpin
    where
        Fut: Future<Output = Result<T, E>> + Unpin,
    {
        self.timeout_future(fut, Instant::now() + duration)
    }
}

impl Default for TimerProvider {
    fn default() -> Self {
        TimerProvider::new()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown_tx.try_send(()).ok();
    }
}
