use std::sync::Arc;

use arc_swap::ArcSwap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeGenerationId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TransitionAttemptId(u64);

trait Authenticator: Send + Sync {
    fn name(&self) -> &'static str;
}

#[derive(Debug)]
struct DefaultAuthenticator;

impl Authenticator for DefaultAuthenticator {
    fn name(&self) -> &'static str {
        "default"
    }
}

#[derive(Debug)]
struct CustomAuthenticator;

impl Authenticator for CustomAuthenticator {
    fn name(&self) -> &'static str {
        "custom"
    }
}

#[derive(Debug)]
struct SharedComponent;

struct RuntimeGeneration {
    id: RuntimeGenerationId,
    config_marker: u64,
    component_marker: u64,
    provider: Arc<dyn Authenticator>,
    scope_plan_marker: u64,
    shared: Arc<SharedComponent>,
}

impl RuntimeGeneration {
    fn new(id: u64, provider: Arc<dyn Authenticator>, shared: Arc<SharedComponent>) -> Self {
        Self {
            id: RuntimeGenerationId(id),
            config_marker: id,
            component_marker: id,
            provider,
            scope_plan_marker: id,
            shared,
        }
    }
}

#[derive(Clone)]
struct RuntimePublication {
    current: Arc<ArcSwap<RuntimeGeneration>>,
}

impl RuntimePublication {
    fn new(initial: RuntimeGeneration) -> Self {
        Self {
            current: Arc::new(ArcSwap::from_pointee(initial)),
        }
    }

    fn pin(&self) -> RuntimeView {
        RuntimeView {
            generation: self.current.load_full(),
        }
    }

    fn propose(
        &self,
        attempt: TransitionAttemptId,
        candidate: RuntimeGeneration,
    ) -> RuntimeProposal {
        RuntimeProposal {
            attempt,
            base: self.current.load_full(),
            candidate: Arc::new(candidate),
        }
    }

    fn publish(&self, proposal: RuntimeProposal) -> Result<RuntimeView, StaleProposal> {
        let previous = self
            .current
            .compare_and_swap(&proposal.base, Arc::clone(&proposal.candidate));

        if !Arc::ptr_eq(&previous, &proposal.base) {
            return Err(StaleProposal {
                attempt: proposal.attempt,
                base: proposal.base.id,
                current: previous.id,
            });
        }

        Ok(RuntimeView {
            generation: proposal.candidate,
        })
    }

    fn live_authenticator(&self) -> LiveAuthenticator {
        LiveAuthenticator {
            publication: self.clone(),
        }
    }

    async fn open_scope(
        &self,
        pinned: tokio::sync::oneshot::Sender<RuntimeGenerationId>,
        resume: tokio::sync::oneshot::Receiver<()>,
    ) -> OpenScope {
        let view = self.pin();
        let _ = pinned.send(view.id());

        resume.await.expect("scope opening resumes");

        OpenScope { view }
    }
}

#[derive(Clone)]
struct RuntimeView {
    generation: Arc<RuntimeGeneration>,
}

impl RuntimeView {
    fn id(&self) -> RuntimeGenerationId {
        self.generation.id
    }

    fn markers(&self) -> (u64, u64, u64) {
        (
            self.generation.config_marker,
            self.generation.component_marker,
            self.generation.scope_plan_marker,
        )
    }

    fn authenticator(&self) -> Arc<dyn Authenticator> {
        Arc::clone(&self.generation.provider)
    }

    fn shared_component(&self) -> &Arc<SharedComponent> {
        &self.generation.shared
    }
}

struct RuntimeProposal {
    attempt: TransitionAttemptId,
    base: Arc<RuntimeGeneration>,
    candidate: Arc<RuntimeGeneration>,
}

#[derive(Debug, PartialEq, Eq)]
struct StaleProposal {
    attempt: TransitionAttemptId,
    base: RuntimeGenerationId,
    current: RuntimeGenerationId,
}

struct LiveAuthenticator {
    publication: RuntimePublication,
}

impl LiveAuthenticator {
    fn snapshot(&self) -> Arc<dyn Authenticator> {
        self.publication.pin().authenticator()
    }
}

struct OpenScope {
    view: RuntimeView,
}

impl OpenScope {
    fn id(&self) -> RuntimeGenerationId {
        self.view.id()
    }

    fn markers(&self) -> (u64, u64, u64) {
        self.view.markers()
    }

    fn authenticator(&self) -> Arc<dyn Authenticator> {
        self.view.authenticator()
    }
}

#[cfg(test)]
mod tests;
