use upwell::daemon::App;
use upwell::{MemoryClient, MemoryConnectionHandle};

pub(crate) use upwell_test_utils::{AbortOnDropTask, deadline};

/// Owns a memory transport daemon and its client-side accept handle.
#[allow(dead_code)]
pub(crate) struct MemoryServer {
    client: Option<MemoryClient>,
    task: AbortOnDropTask<upwell::daemon::Result<()>>,
}

#[allow(dead_code)]
impl MemoryServer {
    pub(crate) fn start(app: App) -> Self {
        let (client, transport) = MemoryClient::pair();
        let task = AbortOnDropTask::spawn("memory daemon", app.serve(transport));

        Self {
            client: Some(client),
            task,
        }
    }

    pub(crate) async fn connect(&self) -> MemoryConnectionHandle {
        let client = self.client.as_ref().expect("memory client is present");

        deadline("memory client connect", client.connect())
            .await
            .expect("connect memory client")
    }

    pub(crate) async fn shutdown<I>(mut self, connections: I)
    where
        I: IntoIterator<Item = MemoryConnectionHandle>,
    {
        connections.into_iter().for_each(drop);
        drop(self.client.take());

        self.task
            .join()
            .await
            .expect("memory daemon stops without error");
    }
}
