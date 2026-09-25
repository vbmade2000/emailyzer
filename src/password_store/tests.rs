#[cfg(test)]
pub(crate) mod password_store_tests {
    use keyring_core::mock;
    use std::sync::atomic::{AtomicI64, Ordering};
    use tokio::sync::{Mutex, MutexGuard};

    use crate::password_store::{
        SERVICE_NAME, delete_password, entry_for, get_password, store_password,
    };

    /// Serializes the tests in this module (and any other test module that touches the
    /// process-global credential store, e.g. `providers::tests`): each test takes this
    /// lock and installs a fresh mock store to stay hermetic (isolated from the real OS
    /// keyring and from other tests).
    ///
    /// A `tokio::sync::Mutex` is used (instead of `std::sync::Mutex`) so the guard can be
    /// held across `.await` points in `#[tokio::test]`s (e.g. in `providers::tests`)
    /// without tripping Clippy's `await_holding_lock` lint, which is specifically about
    /// std's non-async-aware mutex.
    pub(crate) static TEST_LOCK: Mutex<()> = Mutex::const_new(());

    /// Generates provider ids far away from real database rowids so tests can never
    /// collide with real stored passwords, even if the mock setup were bypassed.
    static NEXT_ID: AtomicI64 = AtomicI64::new(1_000_000_000);

    pub(crate) fn next_provider_id() -> i64 {
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    }

    /// Installs a fresh in-memory mock credential store, for use by synchronous (`#[test]`)
    /// tests. The returned guard keeps the tests serialized and must be held for the whole
    /// test.
    pub(crate) fn setup() -> MutexGuard<'static, ()> {
        let guard = TEST_LOCK.blocking_lock();
        install_mock_store();
        guard
    }

    /// Async equivalent of `setup()`, for use by `#[tokio::test]` tests (e.g. in
    /// `providers::tests`), where `blocking_lock()` would panic since it can't be called
    /// from within a Tokio runtime.
    pub(crate) async fn setup_async() -> MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().await;
        install_mock_store();
        guard
    }

    fn install_mock_store() {
        // Force the one-time platform store initialization *before* installing the
        // mock, so the platform init can never overwrite the mock afterwards.
        let _ = keyring::Entry::store_status();
        keyring_core::set_default_store(mock::Store::new().expect("mock store installs"));
    }

    /// Inject a one-shot error into the mock credential backing `provider_id`.
    pub(crate) fn inject_error(provider_id: i64, error: keyring::Error) {
        let entry = entry_for(provider_id).expect("entry builds on mock store");
        let cred = entry
            .as_any()
            .downcast_ref::<mock::Cred>()
            .expect("mock credential");
        cred.set_error(error);
    }

    #[test]
    fn store_and_get_password_roundtrip() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "s3cr3t").unwrap();
        assert_eq!(get_password(id).unwrap(), "s3cr3t");
    }

    #[test]
    fn get_password_without_entry_reports_helpful_error() {
        let _guard = setup();
        let err = get_password(next_provider_id()).unwrap_err();
        assert!(
            err.to_string().contains("No stored password"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn store_password_overwrites_existing_password() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "first").unwrap();
        store_password(id, "second").unwrap();
        assert_eq!(get_password(id).unwrap(), "second");
    }

    #[test]
    fn delete_password_removes_stored_password() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "s3cr3t").unwrap();
        delete_password(id).unwrap();
        let err = get_password(id).unwrap_err();
        assert!(
            err.to_string().contains("No stored password"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn delete_password_is_ok_when_nothing_stored() {
        let _guard = setup();
        delete_password(next_provider_id()).unwrap();
    }

    #[test]
    fn delete_password_twice_is_ok() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "s3cr3t").unwrap();
        delete_password(id).unwrap();
        delete_password(id).unwrap();
    }

    #[test]
    fn passwords_are_isolated_between_provider_ids() {
        let _guard = setup();
        let first = next_provider_id();
        let second = next_provider_id();
        store_password(first, "first-secret").unwrap();
        // The other provider is unaffected ...
        assert!(get_password(second).is_err());
        store_password(second, "second-secret").unwrap();
        assert_eq!(get_password(first).unwrap(), "first-secret");
        assert_eq!(get_password(second).unwrap(), "second-secret");
    }

    #[test]
    fn entries_are_keyed_by_provider_id() {
        let _guard = setup();
        let entry = entry_for(42).unwrap();
        assert_eq!(
            entry.get_specifiers(),
            Some((SERVICE_NAME.to_string(), "42".to_string()))
        );
    }

    #[test]
    fn store_and_get_empty_password() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "").unwrap();
        assert_eq!(get_password(id).unwrap(), "");
    }

    #[test]
    fn store_and_get_unicode_password() {
        let _guard = setup();
        let id = next_provider_id();
        let password = "pässwörd🔑 s3cr3t!@#$%^&*()_+-=[]{}|;':\",./<>?`~\t\n";
        store_password(id, password).unwrap();
        assert_eq!(get_password(id).unwrap(), password);
    }

    #[test]
    fn store_and_get_long_password() {
        let _guard = setup();
        let id = next_provider_id();
        let password = "x".repeat(10_000);
        store_password(id, &password).unwrap();
        assert_eq!(get_password(id).unwrap(), password);
    }

    #[test]
    fn get_password_maps_injected_no_entry_to_helpful_error() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "s3cr3t").unwrap();
        inject_error(id, keyring::Error::NoEntry);
        let err = get_password(id).unwrap_err();
        assert!(
            err.to_string().contains("No stored password"),
            "unexpected error: {err}"
        );
        // The injected error is one-shot: the stored value is untouched.
        assert_eq!(get_password(id).unwrap(), "s3cr3t");
    }

    #[test]
    fn get_password_propagates_non_no_entry_errors() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "s3cr3t").unwrap();
        inject_error(id, keyring::Error::TooLong("service".to_string(), 3));
        let err = get_password(id).unwrap_err();
        assert!(
            !err.to_string().contains("No stored password"),
            "unexpected friendly error: {err}"
        );
    }

    #[test]
    fn store_password_propagates_store_errors() {
        let _guard = setup();
        let id = next_provider_id();
        inject_error(
            id,
            keyring::Error::Invalid("mock".to_string(), "boom".to_string()),
        );
        assert!(store_password(id, "s3cr3t").is_err());
        // The failed store left no password behind.
        assert!(get_password(id).is_err());
    }

    #[test]
    fn delete_password_maps_injected_no_entry_to_ok() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "s3cr3t").unwrap();
        inject_error(id, keyring::Error::NoEntry);
        delete_password(id).unwrap();
    }

    #[test]
    fn delete_password_propagates_non_no_entry_errors() {
        let _guard = setup();
        let id = next_provider_id();
        store_password(id, "s3cr3t").unwrap();
        inject_error(
            id,
            keyring::Error::Invalid("mock".to_string(), "boom".to_string()),
        );
        assert!(delete_password(id).is_err());
    }

    #[test]
    fn operations_fail_without_a_credential_store() {
        let _guard = setup();
        keyring_core::unset_default_store();
        let id = next_provider_id();
        assert!(store_password(id, "s3cr3t").is_err());
        assert!(get_password(id).is_err());
        assert!(delete_password(id).is_err());
    }
}
