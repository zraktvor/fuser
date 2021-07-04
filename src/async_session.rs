//! Filesystem session
//!
//! A session runs a filesystem implementation while it is being mounted to a specific mount
//! point. A session begins by mounting the filesystem and ends by unmounting it. While the
//! filesystem is mounted, the session loop receives, dispatches and replies to kernel requests
//! for filesystem operations under its mount point.
//!
//! You asynchronously wait for this session to umount.
//! An example usage would be, to wait for either ctrl-c or umount:
//! ```rust
//! use std::thread;
//! use tokio::runtime::Builder;
//! use fuser::Filesystem;
//! use std::path::PathBuf;
//!
//! fn mount_and_await<'a, FS: Filesystem + Send + 'static + 'a>(mountpoint: PathBuf, fs: FS) {
//!     thread::spawn(move || {
//!         let umount = fuser::spawn_async_mount(fs, &mountpoint,&[]).expect("spawn filesystem");
//!         let c = tokio::signal::ctrl_c();
//!         println!("Waiting for Ctrl-C...");
//!         let rt = Builder::new_current_thread().enable_io().build().expect("build tokio runtime");
//!         rt.block_on(async move {tokio::select! {
//!             _ = c => {}
//!             _ = umount.await_umount() => {}
//!         }});
//!     }).join().unwrap();
//! }
//!
//! ```

use std::fmt;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::io;

use crate::{Filesystem, Session};
use tokio::sync::oneshot::{Receiver, channel};
use crate::mnt::Mount;

/// The background session data structure
pub struct AsyncBackgroundSession {
    /// Path of the mounted filesystem
    pub mountpoint: PathBuf,
    /// Thread guard of the background session
    pub guard: JoinHandle<io::Result<()>>,
    /// Ensures the filesystem is unmounted when the session ends
    _mount: Mount,
    /// provides a method to find out, whether the fs was umonuted otherwise (f.e. with `fusermount -u`)
    _receiver: Receiver<()>,
}

impl AsyncBackgroundSession {
    /// Create a new background session for the given session by running its
    /// session loop in a background thread. If the returned handle is dropped,
    /// the filesystem is unmounted and the given session ends.
    pub fn new<FS: Filesystem + Send + 'static>(
        mut se: Session<FS>,
    ) -> io::Result<AsyncBackgroundSession> {
        let mountpoint = se.mountpoint().to_path_buf();
        // Take the fuse_session, so that we can unmount it
        let mount = std::mem::take(&mut se.mount);
        let (s, r) = channel();
        let mount = mount.ok_or_else(|| io::Error::from_raw_os_error(libc::ENODEV))?;
        let guard = thread::spawn(move || {
            let mut se = se;
            let res = se.run();
            // ignore the error. There is no need to send anything if the channel was closed.
            let _ = s.send(());
            res
        });
        Ok(AsyncBackgroundSession {
            mountpoint,
            guard,
            _mount: mount,
            _receiver: r,
        })
    }

    /// Unmount the filesystem and join the background thread.
    pub fn join(self) {
        let Self {
            mountpoint: _,
            guard,
            _mount,
            _receiver: _,
        } = self;
        drop(_mount);
        guard.join().unwrap().unwrap();
    }

    /// Tests, whether the filesystem was mounted otherwise (f.e. by `fusermount -u`).
    /// Returns true, if the filesystem was unmounted.
    pub async fn await_umount(self) {
        // closing was also caused by unmounting
        let _ = self._receiver.await;
    }
}

// replace with #[derive(Debug)] if Debug ever gets implemented for
// thread_scoped::JoinGuard
impl<'a> fmt::Debug for AsyncBackgroundSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "BackgroundSession {{ mountpoint: {:?}, guard: JoinGuard<()> }}",
            self.mountpoint
        )
    }
}
