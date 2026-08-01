//! Block notification boundary. Notifications only trigger polling and never carry consensus data.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

/// Wakeup source used by the ingestion loop.
#[async_trait]
pub trait Wakeup: Send {
    /// Wait until the worker should poll Bitcoin Core again.
    async fn wait(&mut self) -> Result<()>;
}

/// Timer-based wakeup used by default and as the reference fallback.
pub struct PollWakeup {
    interval: Duration,
}

impl PollWakeup {
    /// Construct a timer wakeup.
    pub const fn new(interval: Duration) -> Self {
        Self { interval }
    }
}

#[async_trait]
impl Wakeup for PollWakeup {
    async fn wait(&mut self) -> Result<()> {
        tokio::time::sleep(self.interval).await;
        Ok(())
    }
}

/// Optional rawblock ZMQ wakeup. Raw bytes are discarded and blocks are fetched from RPC.
#[cfg(feature = "zmq")]
pub struct ZmqWakeup {
    socket: zeromq::SubSocket,
}

#[cfg(feature = "zmq")]
impl ZmqWakeup {
    /// Connect and subscribe only to the Bitcoin Core `rawblock` topic.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        use zeromq::Socket as _;

        let mut socket = zeromq::SubSocket::new();
        socket.connect(endpoint).await?;
        socket.subscribe("rawblock").await?;
        Ok(Self { socket })
    }
}

#[cfg(feature = "zmq")]
#[async_trait]
impl Wakeup for ZmqWakeup {
    async fn wait(&mut self) -> Result<()> {
        use zeromq::SocketRecv as _;

        let _message = self.socket.recv().await?;
        Ok(())
    }
}
