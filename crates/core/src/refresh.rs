//! Runs providers concurrently (one short-lived thread each) and folds the
//! results into a [`Snapshot`]. Providers are isolated: one failing or slow
//! provider never affects the others.

use crate::snapshot::Snapshot;
use crate::{Limit, Provider, ProviderError};

pub type FetchResult = (&'static str, &'static str, Result<Vec<Limit>, ProviderError>);

/// Fetches all given providers in parallel. Blocks until all finish
/// (each provider bounds itself with the HTTP timeout).
pub fn fetch_all(providers: &[&dyn Provider]) -> Vec<FetchResult> {
    std::thread::scope(|s| {
        let handles: Vec<_> = providers
            .iter()
            .map(|p| {
                let p = *p;
                (p.id(), p.name(), s.spawn(move || p.fetch()))
            })
            .collect();
        handles
            .into_iter()
            .map(|(id, name, h)| {
                let r = h
                    .join()
                    .unwrap_or_else(|_| Err(ProviderError::Unavailable("provider crashed".into())));
                (id, name, r)
            })
            .collect()
    })
}

/// Applies results to the snapshot at `now`.
pub fn apply(snapshot: &mut Snapshot, results: Vec<FetchResult>, now: i64) {
    for (id, name, r) in results {
        snapshot.apply(id, name, r, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Unit;

    struct Fake(&'static str, bool);
    impl Provider for Fake {
        fn id(&self) -> &'static str {
            self.0
        }
        fn name(&self) -> &'static str {
            self.0
        }
        fn fetch(&self) -> Result<Vec<Limit>, ProviderError> {
            if self.1 {
                Ok(vec![Limit {
                    provider: self.0.into(),
                    account_label: None,
                    label: "x".into(),
                    used: 1.0,
                    limit: 2.0,
                    unit: Unit::Requests,
                    window_start: None,
                    resets_at: None,
                }])
            } else {
                Err(ProviderError::Network("down".into()))
            }
        }
    }

    #[test]
    fn isolates_failures() {
        let (a, b) = (Fake("a", true), Fake("b", false));
        let mut snap = Snapshot::default();
        apply(&mut snap, fetch_all(&[&a, &b]), 10);
        assert_eq!(snap.provider("a").unwrap().limits.len(), 1);
        assert!(snap.provider("b").unwrap().is_unavailable());
    }
}
