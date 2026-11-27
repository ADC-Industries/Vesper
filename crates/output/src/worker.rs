//! A bounded background worker that receives copies of frames, used by sinks whose work
//! (encoding, writing to a pipe) must never stall the render loop.
//!
//! The caller asks for a buffer with [`Worker::try_buffer`]; when every buffer is in flight
//! (one being processed plus `depth` queued) it gets `None` and drops the frame without even
//! copying it. Buffers come back through a return channel, so steady state allocates nothing.

use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use anyhow::{Result, anyhow};

/// Work done on the worker thread.
pub(crate) trait Task<M>: Send + 'static {
    /// Handles one queued frame. An error stops the worker; the next submit reports it.
    fn process(&mut self, data: &[u8], meta: M) -> Result<()>;
    /// Called once when the queue is closed (not after a `process` error).
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

pub(crate) struct Worker<M: Send + 'static> {
    tx: Option<SyncSender<(Vec<u8>, M)>>,
    returned: Receiver<Vec<u8>>,
    spare: Vec<Vec<u8>>,
    allocated: usize,
    max_buffers: usize,
    handle: Option<JoinHandle<Result<()>>>,
    /// Frames dropped because the worker was busy.
    pub dropped: u64,
}

impl<M: Send + 'static> Worker<M> {
    /// Spawns the worker thread. `depth` is the number of frames that may wait in the queue
    /// while one is being processed (at least 1).
    pub fn spawn(thread_name: &str, depth: usize, mut task: impl Task<M>) -> Result<Self> {
        let depth = depth.max(1);
        let (tx, rx) = mpsc::sync_channel::<(Vec<u8>, M)>(depth);
        let (ret_tx, returned) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name(thread_name.to_owned())
            .spawn(move || -> Result<()> {
                for (buf, meta) in rx {
                    let r = task.process(&buf, meta);
                    let _ = ret_tx.send(buf);
                    r?;
                }
                task.finish()
            })?;
        Ok(Worker {
            tx: Some(tx),
            returned,
            spare: Vec::new(),
            allocated: 0,
            max_buffers: depth + 1,
            handle: Some(handle),
            dropped: 0,
        })
    }

    /// A free buffer, or `None` when all buffers are in flight (the frame should be dropped).
    pub fn try_buffer(&mut self) -> Option<Vec<u8>> {
        while let Ok(b) = self.returned.try_recv() {
            self.spare.push(b);
        }
        if let Some(b) = self.spare.pop() {
            return Some(b);
        }
        if self.allocated < self.max_buffers {
            self.allocated += 1;
            return Some(Vec::new());
        }
        None
    }

    /// Queues a filled buffer. Returns `Ok(true)` if queued, `Ok(false)` if dropped because the
    /// queue was full, or the worker's error if it has stopped.
    pub fn submit(&mut self, buf: Vec<u8>, meta: M) -> Result<bool> {
        let Some(tx) = &self.tx else {
            return Err(anyhow!("sink is already finished"));
        };
        match tx.try_send((buf, meta)) {
            Ok(()) => Ok(true),
            Err(TrySendError::Full((buf, _))) => {
                self.spare.push(buf);
                self.dropped += 1;
                Ok(false)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.tx = None;
                Err(self
                    .join()
                    .err()
                    .unwrap_or_else(|| anyhow!("worker stopped")))
            }
        }
    }

    /// Records a frame dropped before submit (no buffer available).
    pub fn note_dropped(&mut self) {
        self.dropped += 1;
    }

    /// True once the worker thread has exited (e.g. after an error).
    #[cfg(test)]
    pub fn is_stopped(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// If the worker thread has exited with an error, returns it.
    pub fn check(&mut self) -> Result<()> {
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            self.tx = None;
            return self.join();
        }
        Ok(())
    }

    /// Closes the queue, waits for queued frames to be processed and returns the outcome.
    pub fn finish(&mut self) -> Result<()> {
        self.tx = None;
        self.join()
    }

    /// Test helper: waits until every buffer is back from the worker thread.
    #[cfg(test)]
    pub fn wait_idle(&mut self) {
        while self.spare.len() < self.allocated && !self.is_stopped() {
            if let Ok(b) = self
                .returned
                .recv_timeout(std::time::Duration::from_millis(10))
            {
                self.spare.push(b);
            }
        }
    }

    fn join(&mut self) -> Result<()> {
        match self.handle.take() {
            Some(h) => h
                .join()
                .map_err(|_| anyhow!("worker thread panicked"))
                .and_then(|r| r),
            None => Ok(()),
        }
    }
}

impl<M: Send + 'static> Drop for Worker<M> {
    fn drop(&mut self) {
        if let Err(e) = self.finish() {
            log::warn!("output worker finished with an error: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct Slow(Arc<Mutex<Vec<u8>>>);
    impl Task<u8> for Slow {
        fn process(&mut self, data: &[u8], meta: u8) -> Result<()> {
            std::thread::sleep(Duration::from_millis(30));
            self.0.lock().unwrap().push(meta);
            anyhow::ensure!(data.len() == 1, "bad len");
            anyhow::ensure!(meta != 99, "boom");
            Ok(())
        }
    }

    #[test]
    fn drops_when_busy_and_drains_on_finish() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut w = Worker::spawn("test-worker", 2, Slow(seen.clone())).unwrap();
        let mut queued = 0;
        for i in 0..20u8 {
            match w.try_buffer() {
                Some(mut b) => {
                    b.clear();
                    b.push(i);
                    if w.submit(b, i).unwrap() {
                        queued += 1;
                    }
                }
                None => w.note_dropped(),
            }
        }
        assert!(w.dropped > 0, "a slow worker must cause drops");
        assert!(queued <= 3 + 1);
        w.finish().unwrap();
        assert_eq!(seen.lock().unwrap().len(), queued);
    }

    #[test]
    fn worker_error_surfaces_on_next_submit() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut w = Worker::spawn("test-worker", 1, Slow(seen)).unwrap();
        let b = w.try_buffer().unwrap();
        assert!(w.submit(vec![0], 99).unwrap());
        drop(b);
        while !w.is_stopped() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let err = w.check().unwrap_err();
        assert!(err.to_string().contains("boom"));
        assert!(w.submit(vec![0], 1).is_err());
    }
}
