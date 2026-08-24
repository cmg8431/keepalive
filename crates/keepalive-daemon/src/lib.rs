pub mod clamshell;
mod heartbeat;
mod http;
mod notify;
mod power;
mod server;
mod sessions;

use anyhow::Result;

pub fn run() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(server::serve())
}
