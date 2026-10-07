#[cfg(test)]
mod providers_tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::cli::{AddProviderArgs, DefaultProviderArgs, DeleteProviderArgs};
    use crate::database::{DatabaseLocation, DatabaseManager};
    use crate::password_store::get_password;
    use crate::password_store::tests::password_store_tests::{inject_error, setup_async};
    use crate::providers::providers::{
        add_provider, delete_provider, list_providers, set_default_provider,
    };

    async fn test_db() -> DatabaseManager {
        DatabaseManager::new(DatabaseLocation::Memory)
            .await
            .expect("in-memory test database")
    }

    /// Writes `contents` to a fresh temp file and returns its path, so tests can pass it as
    /// `--password-file` without touching the real filesystem outside of tmp.
    fn write_password_file(contents: &str) -> std::path::PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "emailyzer-providers-test-{}-{}.txt",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn add_args(name: &str, password_path: &std::path::Path) -> AddProviderArgs {
        AddProviderArgs {
            name: name.to_string(),
            url: "imap.example.com".to_string(),
            port: 993,
            username: "user@example.com".to_string(),
            password_file: password_path.to_string_lossy().to_string(),
            inbox_label: "INBOX".to_string(),
            sent_label: "SENT".to_string(),
        }
    }

    #[tokio::test]
    async fn add_provider_rejects_empty_name() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let pw = write_password_file("secret");
        assert!(add_provider(add_args("", &pw), &db).await.is_err());
    }

    #[tokio::test]
    async fn add_provider_stores_provider_and_password() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let pw = write_password_file("secret");
        let args = add_args("p1", &pw);

        add_provider(args, &db).await.unwrap();

        assert!(db.provider_exists("p1".to_string()).await.unwrap());
        let id = db.get_provider_id("p1".to_string()).await.unwrap().unwrap();
        assert_eq!(get_password(id).unwrap(), "secret");
    }

    #[tokio::test]
    async fn add_provider_rolls_back_db_row_when_keyring_store_fails() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let pw = write_password_file("secret");

        // A fresh in-memory database always assigns id 1 to the first inserted row, so the
        // keyring error can be injected for that id ahead of time.
        inject_error(
            1,
            keyring::Error::Invalid("mock".to_string(), "boom".to_string()),
        );

        let result = add_provider(add_args("p1", &pw), &db).await;
        assert!(result.is_err());
        assert!(!db.provider_exists("p1".to_string()).await.unwrap());
    }

    #[tokio::test]
    async fn delete_provider_rejects_empty_name() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let args = DeleteProviderArgs {
            name: String::new(),
            force: true,
        };
        assert!(delete_provider(args, &db).await.is_err());
    }

    #[tokio::test]
    async fn delete_provider_not_found_returns_error() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let args = DeleteProviderArgs {
            name: "missing".to_string(),
            force: true,
        };
        assert!(delete_provider(args, &db).await.is_err());
    }

    #[tokio::test]
    async fn delete_provider_without_force_bails_when_not_interactive() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let pw = write_password_file("secret");
        add_provider(add_args("p1", &pw), &db).await.unwrap();

        let args = DeleteProviderArgs {
            name: "p1".to_string(),
            force: false,
        };
        // Force "non-interactive" regardless of whether the test binary's stdin happens to
        // be a real TTY (e.g. when run directly in a terminal), so this refuses rather than
        // hanging on an interactive confirmation prompt.
        crate::util::set_stdin_interactive_override(Some(false));
        let result = delete_provider(args, &db).await;
        crate::util::set_stdin_interactive_override(None);

        assert!(result.is_err());
        assert!(db.provider_exists("p1".to_string()).await.unwrap());
    }

    #[tokio::test]
    async fn delete_provider_force_removes_provider_and_password() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let pw = write_password_file("secret");
        add_provider(add_args("p1", &pw), &db).await.unwrap();
        let id = db.get_provider_id("p1".to_string()).await.unwrap().unwrap();

        let args = DeleteProviderArgs {
            name: "p1".to_string(),
            force: true,
        };
        delete_provider(args, &db).await.unwrap();

        assert!(!db.provider_exists("p1".to_string()).await.unwrap());
        assert!(get_password(id).is_err());
    }

    #[tokio::test]
    async fn list_providers_with_no_providers_is_ok() {
        let _guard = setup_async().await;
        let db = test_db().await;
        assert!(list_providers(&db).await.is_ok());
    }

    #[tokio::test]
    async fn list_providers_with_providers_is_ok() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let pw = write_password_file("secret");
        add_provider(add_args("p1", &pw), &db).await.unwrap();
        assert!(list_providers(&db).await.is_ok());
    }

    #[tokio::test]
    async fn set_default_provider_rejects_empty_name() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let args = DefaultProviderArgs {
            name: String::new(),
        };
        assert!(set_default_provider(args, &db).await.is_err());
    }

    #[tokio::test]
    async fn set_default_provider_not_found_returns_error() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let args = DefaultProviderArgs {
            name: "missing".to_string(),
        };
        assert!(set_default_provider(args, &db).await.is_err());
    }

    #[tokio::test]
    async fn set_default_provider_success_updates_setting() {
        let _guard = setup_async().await;
        let db = test_db().await;
        let pw = write_password_file("secret");
        add_provider(add_args("p1", &pw), &db).await.unwrap();

        let args = DefaultProviderArgs {
            name: "p1".to_string(),
        };
        set_default_provider(args, &db).await.unwrap();

        assert_eq!(
            db.get_default_provider_opt().await.unwrap(),
            Some("p1".to_string())
        );
    }
}
