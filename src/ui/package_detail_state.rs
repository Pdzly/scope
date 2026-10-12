//! Request identity and short-lived cache shared by all package sources.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::package::InstalledPackage;
use crate::domain::package_details::PackageDetails;

const CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct PackageDetailState {
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    cache: HashMap<(String, String), (Instant, PackageDetails)>,
    partial: Option<(String, String, PackageDetails)>,
}

pub(super) struct DetailRequest {
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
}

impl PackageDetailState {
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.partial = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn clear(&mut self) {
        self.cancel();
        self.cache.clear();
    }

    pub fn begin(&mut self, pkg: &InstalledPackage) -> Option<DetailRequest> {
        self.cancel();
        if self
            .cache
            .get(&(pkg.key.clone(), pkg.version.clone()))
            .is_some_and(|(created, details)| {
                created.elapsed() < CACHE_TTL && !details.items.is_empty()
            })
        {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        Some(DetailRequest {
            generation: self.generation,
            cancel,
        })
    }

    pub fn accept(
        &mut self,
        generation: u64,
        pkg: &InstalledPackage,
        details: PackageDetails,
        complete: bool,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        if !complete {
            // Preserve a complete cached layout while refreshing its values.
            if self
                .cache
                .get(&(pkg.key.clone(), pkg.version.clone()))
                .is_some_and(|(_, details)| !details.items.is_empty())
            {
                return false;
            }
            self.partial = Some((pkg.key.clone(), pkg.version.clone(), details));
            return true;
        }
        if self.cache.len() >= 32 {
            if let Some(oldest) = self
                .cache
                .iter()
                .min_by_key(|(_, (created, _))| *created)
                .map(|(key, _)| key.clone())
            {
                self.cache.remove(&oldest);
            }
        }
        self.cache.insert(
            (pkg.key.clone(), pkg.version.clone()),
            (Instant::now(), details),
        );
        self.cancel = None;
        self.partial = None;
        true
    }

    pub fn get(&self, pkg: &InstalledPackage) -> Option<&PackageDetails> {
        if let Some((key, version, details)) = &self.partial {
            if key == &pkg.key && version == &pkg.version {
                return Some(details);
            }
        }
        // Expired complete results stay visible while a fresh inspection runs.
        self.cache
            .get(&(pkg.key.clone(), pkg.version.clone()))
            .map(|(_, details)| details)
    }
}

impl Drop for PackageDetailState {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::package::PackageSource;

    fn details() -> PackageDetails {
        PackageDetails {
            items: vec![crate::domain::package_details::DetailItem {
                label: "Revision 1".into(),
                location: None,
                size: crate::domain::package_details::Measurement {
                    status: Default::default(),
                    bytes: Some(1),
                    complete: true,
                    note: None,
                },
                effect: crate::domain::package_details::DetailEffect::Removed,
            }],
            warnings: vec![],
        }
    }

    #[test]
    fn expired_cache_remains_visible_until_refresh_finishes() {
        let mut state = PackageDetailState::default();
        let pkg = InstalledPackage::new(PackageSource::Snap, "code");
        let request = state.begin(&pkg).unwrap();
        assert!(state.accept(request.generation, &pkg, details(), true));
        state
            .cache
            .get_mut(&(pkg.key.clone(), pkg.version.clone()))
            .unwrap()
            .0 = Instant::now() - CACHE_TTL;
        let refresh = state.begin(&pkg).unwrap();
        assert!(state.get(&pkg).is_some());
        let mut changed = details();
        changed.items[0].size.bytes = Some(2);
        assert!(!state.accept(refresh.generation, &pkg, changed.clone(), false));
        assert_eq!(state.get(&pkg).unwrap().items[0].size.bytes, Some(1));
        assert!(state.accept(refresh.generation, &pkg, changed, true));
        assert_eq!(state.get(&pkg).unwrap().items[0].size.bytes, Some(2));
    }

    #[test]
    fn partial_results_are_not_reused_after_canceling() {
        let mut state = PackageDetailState::default();
        let pkg = InstalledPackage::new(PackageSource::Snap, "code");
        let request = state.begin(&pkg).unwrap();
        assert!(state.accept(request.generation, &pkg, details(), false));
        assert!(state.get(&pkg).is_some());
        state.cancel();
        assert!(state.get(&pkg).is_none());
        let next = state.begin(&pkg).unwrap();
        assert!(!state.accept(request.generation, &pkg, details(), true));
        assert!(state.accept(next.generation, &pkg, details(), true));
    }

    #[test]
    fn late_results_cannot_replace_new_selection() {
        let mut state = PackageDetailState::default();
        let first = InstalledPackage::new(PackageSource::Snap, "code");
        let second = InstalledPackage::new(PackageSource::Snap, "firefox");
        let old = state.begin(&first).unwrap();
        let new = state.begin(&second).unwrap();
        assert!(old.cancel.load(Ordering::Relaxed));
        assert!(!state.accept(old.generation, &first, details(), true));
        assert!(state.accept(new.generation, &second, details(), true));
        assert!(state.get(&first).is_none());
    }

    #[test]
    fn scan_invalidation_rejects_in_flight_results_and_clears_cache() {
        let mut state = PackageDetailState::default();
        let pkg = InstalledPackage::new(PackageSource::Snap, "code");
        let request = state.begin(&pkg).unwrap();
        state.clear();
        assert!(!state.accept(request.generation, &pkg, details(), true));
        let request = state.begin(&pkg).unwrap();
        assert!(state.accept(request.generation, &pkg, details(), true));
        assert!(state.begin(&pkg).is_none());
        state.clear();
        assert!(state.get(&pkg).is_none());
    }

    #[test]
    fn flatpak_user_and_system_details_have_independent_cache_entries() {
        use crate::domain::package::InstallScope;
        let mut state = PackageDetailState::default();
        let user = InstalledPackage::new_scoped(
            PackageSource::Flatpak,
            "org.example.App",
            InstallScope::User,
        );
        let system = InstalledPackage::new_scoped(
            PackageSource::Flatpak,
            "org.example.App",
            InstallScope::System,
        );
        let request = state.begin(&user).unwrap();
        assert!(state.accept(request.generation, &user, details(), true));
        let request = state.begin(&system).unwrap();
        let mut system_details = details();
        system_details.items[0].size.bytes = Some(2);
        assert!(state.accept(request.generation, &system, system_details, true));
        assert_eq!(state.get(&user).unwrap().items[0].size.bytes, Some(1));
        assert_eq!(state.get(&system).unwrap().items[0].size.bytes, Some(2));
    }
}
