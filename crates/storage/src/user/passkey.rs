// Copyright 2024, 2025 New Vector Ltd.
// Copyright 2022-2024 The Matrix.org Foundation C.I.C.
//
// SPDX-License-Identifier: AGPL-3.0-only
// Please see LICENSE in the repository root for full details.

use async_trait::async_trait;
use mas_data_model::{BrowserSession, User, UserPasskey, UserPasskeyChallenge};
use rand_core::RngCore;
use ulid::Ulid;

use crate::{Clock, repository_impl};

/// A [`UserPasskeyRepository`] helps interacting with [`UserPasskey`] saved in the
/// storage backend
#[async_trait]
pub trait UserPasskeyRepository: Send + Sync {
    /// The error type returned by the repository
    type Error;

    /// Lookup an [`UserPasskey`] by its ID
    ///
    /// Returns `None` if no [`UserPasskey`] was found
    ///
    /// # Parameters
    ///
    /// * `id`: The ID of the [`UserPasskey`] to lookup
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] if the underlying repository fails
    async fn lookup(&mut self, id: Ulid) -> Result<Option<UserPasskey>, Self::Error>;

    /// Get all [`UserPasskey`] of a [`User`]
    ///
    /// # Parameters
    ///
    /// * `user`: The [`User`] for whom to lookup the [`UserPasskey`]
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] if the underlying repository fails
    async fn all(&mut self, user: &User) -> Result<Vec<UserPasskey>, Self::Error>;

    /// Create a new [`UserPasskey`] for a [`User`]
    ///
    /// Returns the newly created [`UserPasskey`]
    ///
    /// # Parameters
    ///
    /// * `rng`: The random number generator to use
    /// * `clock`: The clock to use
    /// * `user`: The [`User`] for whom to create the [`UserPasskey`]
    /// * `name`: The name for the [`UserPasskey`]
    /// * `data`: The passkey data of the [`UserPasskey`]
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] if the underlying repository fails
    async fn add(
        &mut self,
        rng: &mut (dyn RngCore + Send),
        clock: &dyn Clock,
        user: &User,
        name: String,
        data: serde_json::Value,
    ) -> Result<UserPasskey, Self::Error>;

    /// Rename a [`UserPasskey`]
    ///
    /// Returns the modified [`UserPasskey`]
    ///
    /// # Parameters
    ///
    /// * `user_passkey`: The [`UserPasskey`] to rename
    /// * `name`: The new name
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying repository fails
    async fn rename(
        &mut self,
        user_passkey: UserPasskey,
        name: String,
    ) -> Result<UserPasskey, Self::Error>;

    /// Update a [`UserPasskey`]
    ///
    /// Returns the modified [`UserPasskey`]
    ///
    /// # Parameters
    ///
    /// * `clock`: The clock to use
    /// * `user_passkey`: The [`UserPasskey`] to update
    /// * `data`: The new passkey data
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying repository fails
    async fn update(
        &mut self,
        clock: &dyn Clock,
        user_passkey: UserPasskey,
        data: serde_json::Value,
    ) -> Result<UserPasskey, Self::Error>;

    /// Delete a [`UserPasskey`]
    ///
    /// # Parameters
    ///
    /// * `user_passkey`: The [`UserPasskey`] to delete
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] if the underlying repository fails
    async fn remove(&mut self, user_passkey: UserPasskey) -> Result<(), Self::Error>;

    /// Add a new [`UserPasskeyChallenge`] for a [`BrowserSession`]
    ///
    /// # Parameters
    ///
    /// * `rng`: The random number generator to use
    /// * `clock`: The clock to use
    /// * `state`: The challenge state to add
    /// * `session`: The [`BrowserSession`] for which to add the
    ///   [`UserPasskeyChallenge`]
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying repository fails
    async fn add_challenge_for_session(
        &mut self,
        rng: &mut (dyn RngCore + Send),
        clock: &dyn Clock,
        state: serde_json::Value,
        session: &BrowserSession,
    ) -> Result<UserPasskeyChallenge, Self::Error>;

    /// Add a new [`UserPasskeyChallenge`]
    ///
    /// # Parameters
    ///
    /// * `rng`: The random number generator to use
    /// * `clock`: The clock to use
    /// * `state`: The challenge state to add
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying repository fails
    async fn add_challenge(
        &mut self,
        rng: &mut (dyn RngCore + Send),
        clock: &dyn Clock,
        state: serde_json::Value,
    ) -> Result<UserPasskeyChallenge, Self::Error>;

    /// Lookup a [`UserPasskeyChallenge`]
    ///
    /// # Parameters
    ///
    /// * `id`: The ID of the [`UserPasskeyChallenge`] to lookup
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying repository fails
    async fn lookup_challenge(
        &mut self,
        id: Ulid,
    ) -> Result<Option<UserPasskeyChallenge>, Self::Error>;

    /// Complete a [`UserPasskeyChallenge`] by using the given code
    ///
    /// Returns the completed [`UserPasskeyChallenge`]
    ///
    /// # Parameters
    ///
    /// * `clock`: The clock to use to generate timestamps
    /// * `challenge`: The [`UserPasskeyChallenge`] to complete
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying repository fails
    async fn complete_challenge(
        &mut self,
        clock: &dyn Clock,
        user_passkey_challenge: UserPasskeyChallenge,
    ) -> Result<UserPasskeyChallenge, Self::Error>;
}

repository_impl!(UserPasskeyRepository:
    async fn lookup(&mut self, id: Ulid) -> Result<Option<UserPasskey>, Self::Error>;
    async fn all(&mut self, user: &User) -> Result<Vec<UserPasskey>, Self::Error>;

    async fn add(
        &mut self,
        rng: &mut (dyn RngCore + Send),
        clock: &dyn Clock,
        user: &User,
        name: String,
        data: serde_json::Value,
    ) -> Result<UserPasskey, Self::Error>;
    async fn rename(
        &mut self,
        user_passkey: UserPasskey,
        name: String,
    ) -> Result<UserPasskey, Self::Error>;
    async fn update(
        &mut self,
        clock: &dyn Clock,
        user_passkey: UserPasskey,
        data: serde_json::Value,
    ) -> Result<UserPasskey, Self::Error>;
    async fn remove(&mut self, user_passkey: UserPasskey) -> Result<(), Self::Error>;

    async fn add_challenge_for_session(
        &mut self,
        rng: &mut (dyn RngCore + Send),
        clock: &dyn Clock,
        state: serde_json::Value,
        session: &BrowserSession,
    ) -> Result<UserPasskeyChallenge, Self::Error>;
    async fn add_challenge(
        &mut self,
        rng: &mut (dyn RngCore + Send),
        clock: &dyn Clock,
        state: serde_json::Value,
    ) -> Result<UserPasskeyChallenge, Self::Error>;

    async fn lookup_challenge(
        &mut self,
        id: Ulid,
    ) -> Result<Option<UserPasskeyChallenge>, Self::Error>;

    async fn complete_challenge(
        &mut self,
        clock: &dyn Clock,
        user_passkey_challenge: UserPasskeyChallenge,
    ) -> Result<UserPasskeyChallenge, Self::Error>;
);
