use std::sync::Arc;

use super::*;

fn generation(
    id: u64,
    provider: Arc<dyn Authenticator>,
    shared: Arc<SharedComponent>,
) -> RuntimeGeneration {
    RuntimeGeneration::new(id, provider, shared)
}

#[test]
fn one_publication_keeps_pinned_reads_generation_consistent() {
    let shared = Arc::new(SharedComponent);
    let publication = RuntimePublication::new(generation(
        0,
        Arc::new(DefaultAuthenticator),
        Arc::clone(&shared),
    ));
    let old = publication.pin();
    let proposal = publication.propose(
        TransitionAttemptId(1),
        generation(1, Arc::new(CustomAuthenticator), Arc::clone(&shared)),
    );

    let committed = publication.publish(proposal).expect("proposal is current");

    assert_eq!(old.id(), RuntimeGenerationId(0));
    assert_eq!(old.markers(), (0, 0, 0));
    assert_eq!(old.authenticator().name(), "default");
    assert_eq!(committed.id(), RuntimeGenerationId(1));
    assert_eq!(committed.markers(), (1, 1, 1));
    assert_eq!(committed.authenticator().name(), "custom");
    assert!(Arc::ptr_eq(
        old.shared_component(),
        committed.shared_component()
    ));
}

#[test]
fn live_provider_switches_while_fixed_snapshots_remain_stable() {
    let shared = Arc::new(SharedComponent);
    let publication = RuntimePublication::new(generation(
        0,
        Arc::new(DefaultAuthenticator),
        Arc::clone(&shared),
    ));
    let live = publication.live_authenticator();
    let fixed = live.snapshot();
    let custom = publication.propose(
        TransitionAttemptId(1),
        generation(1, Arc::new(CustomAuthenticator), Arc::clone(&shared)),
    );

    publication
        .publish(custom)
        .expect("custom proposal commits");

    assert_eq!(live.snapshot().name(), "custom");
    assert_eq!(fixed.name(), "default");

    let default = publication.propose(
        TransitionAttemptId(2),
        generation(2, Arc::new(DefaultAuthenticator), shared),
    );

    publication
        .publish(default)
        .expect("default proposal commits");

    assert_eq!(live.snapshot().name(), "default");
    assert_eq!(fixed.name(), "default");
}

#[test]
fn stale_proposal_cannot_overwrite_a_newer_generation() {
    let shared = Arc::new(SharedComponent);
    let publication = RuntimePublication::new(generation(
        0,
        Arc::new(DefaultAuthenticator),
        Arc::clone(&shared),
    ));
    let winner = publication.propose(
        TransitionAttemptId(1),
        generation(1, Arc::new(CustomAuthenticator), Arc::clone(&shared)),
    );
    let stale = publication.propose(
        TransitionAttemptId(2),
        generation(2, Arc::new(DefaultAuthenticator), shared),
    );

    publication.publish(winner).expect("winner commits");
    let error = match publication.publish(stale) {
        Ok(_) => panic!("stale proposal commits"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        StaleProposal {
            attempt: TransitionAttemptId(2),
            base: RuntimeGenerationId(0),
            current: RuntimeGenerationId(1),
        }
    );
    assert_eq!(publication.pin().id(), RuntimeGenerationId(1));
    assert_eq!(publication.pin().authenticator().name(), "custom");
}

#[tokio::test]
async fn scope_opening_uses_one_generation_across_publication() {
    let shared = Arc::new(SharedComponent);
    let publication = RuntimePublication::new(generation(
        0,
        Arc::new(DefaultAuthenticator),
        Arc::clone(&shared),
    ));
    let opening_publication = publication.clone();
    let (pinned_tx, pinned_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    let opening =
        tokio::spawn(async move { opening_publication.open_scope(pinned_tx, resume_rx).await });

    assert_eq!(
        pinned_rx
            .await
            .expect("scope reports its pinned generation"),
        RuntimeGenerationId(0)
    );

    let proposal = publication.propose(
        TransitionAttemptId(1),
        generation(1, Arc::new(CustomAuthenticator), shared),
    );
    publication.publish(proposal).expect("proposal commits");
    resume_tx.send(()).expect("scope task is waiting");

    let old_scope = opening.await.expect("scope task completes");
    let (new_pinned_tx, new_pinned_rx) = tokio::sync::oneshot::channel();
    let (new_resume_tx, new_resume_rx) = tokio::sync::oneshot::channel();
    new_resume_tx
        .send(())
        .expect("new scope has a resume signal");
    let new_scope = publication.open_scope(new_pinned_tx, new_resume_rx).await;

    assert_eq!(old_scope.id(), RuntimeGenerationId(0));
    assert_eq!(old_scope.markers(), (0, 0, 0));
    assert_eq!(old_scope.authenticator().name(), "default");
    assert_eq!(
        new_pinned_rx
            .await
            .expect("scope reports its pinned generation"),
        RuntimeGenerationId(1)
    );
    assert_eq!(new_scope.id(), RuntimeGenerationId(1));
    assert_eq!(new_scope.markers(), (1, 1, 1));
    assert_eq!(new_scope.authenticator().name(), "custom");
}

#[test]
fn old_generation_is_reclaimed_after_the_last_pin_drops() {
    let shared = Arc::new(SharedComponent);
    let publication = RuntimePublication::new(generation(
        0,
        Arc::new(DefaultAuthenticator),
        Arc::clone(&shared),
    ));
    let old = publication.pin();
    let old_generation = Arc::downgrade(&old.generation);
    let proposal = publication.propose(
        TransitionAttemptId(1),
        generation(1, Arc::new(CustomAuthenticator), shared),
    );

    publication.publish(proposal).expect("proposal commits");

    assert!(old_generation.upgrade().is_some());

    drop(old);

    assert!(old_generation.upgrade().is_none());
}
