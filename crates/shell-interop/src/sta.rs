use crossbeam_channel::{Sender, bounded, unbounded};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A pool of threads that each initialise COM as a single-threaded apartment.
#[derive(Clone)]
pub struct StaPool {
    tx: Sender<Job>,
}

impl StaPool {
    pub fn new(name: &str, threads: usize) -> Self {
        let (tx, rx) = unbounded::<Job>();
        for i in 0..threads.max(1) {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("{name}-{i}"))
                .spawn(move || {
                    init_sta();
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                })
                .expect("spawn STA thread");
        }
        Self { tx }
    }

    /// Run `f` on a pool thread and block until it returns.
    pub fn run<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (rtx, rrx) = bounded(1);
        self.tx
            .send(Box::new(move || {
                let _ = rtx.send(f());
            }))
            .expect("STA pool alive");
        rrx.recv().expect("STA job completed")
    }

    /// Fire-and-forget.
    pub fn spawn(&self, f: impl FnOnce() + Send + 'static) {
        let _ = self.tx.send(Box::new(f));
    }
}

/// Initialise COM (STA) on the current thread. Safe to call repeatedly.
pub fn init_sta() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
}
