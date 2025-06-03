// Copyright 2024, 2025 New Vector Ltd.
// Copyright 2021-2024 The Matrix.org Foundation C.I.C.
//
// SPDX-License-Identifier: AGPL-3.0-only
// Please see LICENSE in the repository root for full details.

use std::sync::{Arc, LazyLock};

use mas_data_model::SiteConfig;
use mas_email::Mailer;
use mas_matrix::HomeserverConnection;
use mas_router::UrlBuilder;
use mas_storage::{BoxClock, BoxRepository, RepositoryError, RepositoryFactory, SystemClock};
use mas_storage_pg::PgRepositoryFactory;
use new_queue::QueueRunnerError;
use opentelemetry::metrics::Meter;
use rand::SeedableRng;
use sqlx::{Pool, Postgres};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

mod database;
mod email;
mod matrix;
mod new_queue;
mod recovery;
mod sessions;
mod user;

static METER: LazyLock<Meter> = LazyLock::new(|| {
    let scope = opentelemetry::InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_schema_url(opentelemetry_semantic_conventions::SCHEMA_URL)
        .build();

    opentelemetry::global::meter_with_scope(scope)
});

#[derive(Clone)]
struct State {
    repository_factory: PgRepositoryFactory,
    mailer: Mailer,
    clock: SystemClock,
    homeserver: Arc<dyn HomeserverConnection>,
    url_builder: UrlBuilder,
    site_config: SiteConfig,
}

impl State {
    pub fn new(
        repository_factory: PgRepositoryFactory,
        clock: SystemClock,
        mailer: Mailer,
        homeserver: impl HomeserverConnection + 'static,
        url_builder: UrlBuilder,
        site_config: SiteConfig,
    ) -> Self {
        Self {
            repository_factory,
            mailer,
            clock,
            homeserver: Arc::new(homeserver),
            url_builder,
            site_config,
        }
    }

    pub fn pool(&self) -> Pool<Postgres> {
        self.repository_factory.pool()
    }

    pub fn clock(&self) -> BoxClock {
        Box::new(self.clock.clone())
    }

    pub fn mailer(&self) -> &Mailer {
        &self.mailer
    }

    // This is fine for now, we may move that to a trait at some point.
    #[allow(clippy::unused_self, clippy::disallowed_methods)]
    pub fn rng(&self) -> rand_chacha::ChaChaRng {
        rand_chacha::ChaChaRng::from_rng(rand::thread_rng()).expect("failed to seed rng")
    }

    pub async fn repository(&self) -> Result<BoxRepository, RepositoryError> {
        self.repository_factory.create().await
    }

    pub fn matrix_connection(&self) -> &dyn HomeserverConnection {
        self.homeserver.as_ref()
    }

    pub fn url_builder(&self) -> &UrlBuilder {
        &self.url_builder
    }

    pub fn site_config(&self) -> &SiteConfig {
        &self.site_config
    }
}

/// Initialise the workers.
///
/// # Errors
///
/// This function can fail if the database connection fails.
pub async fn init(
    repository_factory: PgRepositoryFactory,
    mailer: &Mailer,
    homeserver: impl HomeserverConnection + 'static,
    url_builder: UrlBuilder,
    site_config: &SiteConfig,
    cancellation_token: CancellationToken,
    task_tracker: &TaskTracker,
) -> Result<(), QueueRunnerError> {
    let state = State::new(
        repository_factory,
        SystemClock::default(),
        mailer.clone(),
        homeserver,
        url_builder,
        site_config.clone(),
    );
    let mut worker = self::new_queue::QueueWorker::new(state, cancellation_token).await?;

    worker
        .register_handler::<mas_storage::queue::CleanupExpiredTokensJob>()
        .register_handler::<mas_storage::queue::DeactivateUserJob>()
        .register_handler::<mas_storage::queue::DeleteDeviceJob>()
        .register_handler::<mas_storage::queue::ProvisionDeviceJob>()
        .register_handler::<mas_storage::queue::ProvisionUserJob>()
        .register_handler::<mas_storage::queue::ReactivateUserJob>()
        .register_handler::<mas_storage::queue::SendAccountRecoveryEmailsJob>()
        .register_handler::<mas_storage::queue::SendEmailAuthenticationCodeJob>()
        .register_handler::<mas_storage::queue::SyncDevicesJob>()
        .register_handler::<mas_storage::queue::VerifyEmailJob>()
        .register_handler::<mas_storage::queue::ExpireInactiveSessionsJob>()
        .register_handler::<mas_storage::queue::ExpireInactiveCompatSessionsJob>()
        .register_handler::<mas_storage::queue::ExpireInactiveOAuthSessionsJob>()
        .register_handler::<mas_storage::queue::ExpireInactiveUserSessionsJob>()
        .register_handler::<mas_storage::queue::PruneStalePolicyDataJob>()
        .register_handler::<mas_storage::queue::CleanupOldPasskeyChallenges>()
        .add_schedule(
            "cleanup-expired-tokens",
            "0 0 * * * *".parse()?,
            mas_storage::queue::CleanupExpiredTokensJob,
        )
        .add_schedule(
            "expire-inactive-sessions",
            // Run this job every 15 minutes
            "30 */15 * * * *".parse()?,
            mas_storage::queue::ExpireInactiveSessionsJob,
        )
        .add_schedule(
            "prune-stale-policy-data",
            // Run once a day
            "0 0 2 * * *".parse()?,
            mas_storage::queue::PruneStalePolicyDataJob,
        )
        .add_schedule(
            "cleanup-old-passkey-challenges",
            "0 0 * * * *".parse()?,
            mas_storage::queue::CleanupOldPasskeyChallenges,
        );

    task_tracker.spawn(worker.run());

    Ok(())
}
