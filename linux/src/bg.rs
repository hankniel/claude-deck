//! Run blocking work (network, git, disk) off the main thread and finish on it.

use gtk::{gio, glib};

pub fn run<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(Result<T, String>) + 'static,
) {
    glib::spawn_future_local(async move {
        let r = gio::spawn_blocking(move || work().map_err(|e| format!("{e:#}"))).await;
        done(r.unwrap_or_else(|_| Err("the task crashed".into())));
    });
}
