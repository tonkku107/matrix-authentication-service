// Copyright 2025 New Vector Ltd.
//
// SPDX-License-Identifier: AGPL-3.0-only
// Please see LICENSE in the repository root for full details.

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Duration;
use mas_data_model::{BrowserSession, User, UserPasskey, UserPasskeyChallenge};
use mas_matrix::HomeserverConnection;
use mas_storage::{Clock, RepositoryAccess};
use rand::RngCore;
use ulid::Ulid;
use url::Url;
use webauthn_rp::{
    AuthenticatedCredential, Authentication, AuthenticationServerState,
    PublicKeyCredentialCreationOptions, PublicKeyCredentialRequestOptions, RegistrationServerState,
    bin::{Decode, Encode},
    request::{
        DomainOrigin, Port, PublicKeyCredentialDescriptor, RpId, Scheme,
        auth::AuthenticationVerificationOptions,
        register::{PublicKeyCredentialUserEntity, RegistrationVerificationOptions, UserHandle},
    },
    response::{
        CredentialId,
        auth::{error::AuthCeremonyErr, ser_relaxed::AuthenticationRelaxed},
        register::{
            DynamicState, StaticState, error::RegCeremonyErr, ser_relaxed::RegistrationRelaxed,
        },
    },
};

/// User-facing errors
#[derive(Debug, thiserror::Error)]
pub enum WebauthnError {
    #[error(transparent)]
    RegistrationCeremonyError(#[from] RegCeremonyErr),

    #[error(transparent)]
    AuthenticationCeremonyError(#[from] AuthCeremonyErr),

    #[error("The challenge doesn't exist, expired or doesn't belong for this session")]
    InvalidChallenge,

    #[error("Credential already exists")]
    Exists,

    #[error("Authenticator did not include the userHandle in the response")]
    UserHandleMissing,

    #[error("Failed to find a user based on the userHandle")]
    UserNotFound,

    #[error("Failed to find a passkey based on the credential_id")]
    PasskeyNotFound,

    #[error("The passkey belongs to a different user")]
    UserMismatch,
}

#[derive(Clone, Debug)]
pub struct Webauthn {
    rpid: Arc<RpId>,
}

impl Webauthn {
    /// Creates a new instance
    ///
    /// # Errors
    /// If the `public_base` has no valid host domain
    pub fn new(public_base: &Url) -> Result<Self> {
        let host = public_base
            .host_str()
            .context("Public base doesn't have a host")?
            .to_owned();

        let rpid = Arc::new(RpId::Domain(host.try_into()?));

        Ok(Self { rpid })
    }

    #[must_use]
    pub fn get_allowed_origin(&self) -> DomainOrigin {
        let host = (*self.rpid).as_ref();
        if host == "localhost" {
            DomainOrigin {
                scheme: Scheme::Any,
                host,
                port: Port::Any,
            }
        } else {
            DomainOrigin::new(host)
        }
    }

    /// Finds a challenge and does some checks on it
    ///
    /// # Errors
    /// [`WebauthnError::InvalidChallenge`] if the challenge is not found or is
    /// invalid.
    ///
    /// The rest of the anyhow errors should be treated as internal errors
    pub async fn lookup_challenge(
        &self,
        repo: &mut impl RepositoryAccess,
        clock: &impl Clock,
        id: Ulid,
        browser_session: Option<&BrowserSession>,
    ) -> Result<UserPasskeyChallenge> {
        let user_passkey_challenge = repo
            .user_passkey()
            .lookup_challenge(id)
            .await?
            .ok_or(WebauthnError::InvalidChallenge)?;

        // Check that challenge belongs to a browser session if provided or belongs to
        // no session if not provided. If not tied to a session, challenge should
        // be tied by a cookie and checked in the handler
        if user_passkey_challenge.user_session_id != browser_session.map(|s| s.id) {
            return Err(WebauthnError::InvalidChallenge.into());
        }

        // Challenge was already completed
        if user_passkey_challenge.completed_at.is_some() {
            return Err(WebauthnError::InvalidChallenge.into());
        }

        // Challenge has expired
        if clock.now() - user_passkey_challenge.created_at > Duration::hours(1) {
            return Err(WebauthnError::InvalidChallenge.into());
        }

        Ok(user_passkey_challenge)
    }

    /// Creates a passkey registration challenge
    ///
    /// # Returns
    /// 1. The JSON options to `navigator.credentials.create()` on the frontend
    /// 2. The created [`UserPasskeyChallenge`]
    ///
    /// # Errors
    /// Various anyhow errors that should be treated as internal errors
    pub async fn start_passkey_registration(
        &self,
        repo: &mut impl RepositoryAccess,
        rng: &mut (dyn RngCore + Send),
        clock: &impl Clock,
        conn: &impl HomeserverConnection,
        user: &User,
        browser_session: &BrowserSession,
    ) -> Result<(String, UserPasskeyChallenge)> {
        // Get display name or default to username
        let matrix_user = conn.query_user(&conn.mxid(&user.username)).await?;
        let display_name = matrix_user
            .displayname
            .unwrap_or_else(|| user.username.clone());

        // Construct the correct type of user handle...
        let user_handle = UserHandle::<[u8; 16]>::decode(user.id.to_bytes())?;
        let user_handle = UserHandle::<&[u8]>::from(&user_handle);

        let user_entity = PublicKeyCredentialUserEntity {
            name: user.username.as_str().try_into()?,
            id: user_handle,
            display_name: Some(display_name.as_str().try_into()?),
        };

        let exclude_credentials = repo
            .user_passkey()
            .all(user)
            .await?
            .into_iter()
            .map(|v| {
                Ok(PublicKeyCredentialDescriptor {
                    id: serde_json::from_str(&v.credential_id)?,
                    transports: serde_json::from_value(v.transports)?,
                })
            })
            .collect::<Result<Vec<PublicKeyCredentialDescriptor<Vec<u8>>>>>()?;

        let options = PublicKeyCredentialCreationOptions::passkey(
            &self.rpid,
            user_entity,
            exclude_credentials,
        );

        let (server_state, client_state) = options.start_ceremony()?;

        let user_passkey_challenge = repo
            .user_passkey()
            .add_challenge_for_session(rng, clock, server_state.encode()?, browser_session)
            .await?;

        Ok((
            serde_json::to_string(&client_state)?,
            user_passkey_challenge,
        ))
    }

    /// Validates and creates a passkey from a challenge response
    ///
    /// # Errors
    /// [`WebauthnError::Exists`] if the passkey credential the user is trying
    /// to register already exists.
    ///
    /// [`WebauthnError::RegistrationCeremonyError`] if the response from the
    /// user is invalid.
    ///
    /// The rest of the anyhow errors should be treated as internal errors
    pub async fn finish_passkey_registration(
        &self,
        repo: &mut impl RepositoryAccess,
        rng: &mut (dyn RngCore + Send),
        clock: &impl Clock,
        user: &User,
        user_passkey_challenge: UserPasskeyChallenge,
        response: String,
        name: String,
    ) -> Result<UserPasskey> {
        let server_state = RegistrationServerState::decode(&user_passkey_challenge.state)?;

        let response = serde_json::from_str::<RegistrationRelaxed>(&response)?.0;

        let options = RegistrationVerificationOptions::<DomainOrigin, DomainOrigin> {
            allowed_origins: &[self.get_allowed_origin()],
            client_data_json_relaxed: true,
            ..Default::default()
        };

        let user_handle = UserHandle::<[u8; 16]>::decode(user.id.to_bytes())?;
        let user_handle = UserHandle::<&[u8]>::from(&user_handle);

        let credential = server_state
            .verify(&self.rpid, user_handle, &response, &options)
            .map_err(WebauthnError::from)?;

        let cred_id = serde_json::to_string(&credential.id())?;

        // Webauthn requires that credential IDs be unique globally
        if repo.user_passkey().find(&cred_id).await?.is_some() {
            return Err(WebauthnError::Exists.into());
        };

        let user_passkey = repo
            .user_passkey()
            .add(
                rng,
                clock,
                user,
                name,
                cred_id,
                serde_json::to_value(credential.transports())?,
                credential.static_state().encode()?,
                credential.dynamic_state().encode()?.to_vec(),
                credential.metadata().encode()?,
            )
            .await?;

        repo.user_passkey()
            .complete_challenge(clock, user_passkey_challenge)
            .await?;

        Ok(user_passkey)
    }

    /// Creates a passkey authentication challenge
    ///
    /// # Returns
    /// 1. The JSON options to `navigator.credentials.get()` on the frontend
    /// 2. The created [`UserPasskeyChallenge`]
    ///
    /// # Errors
    /// Various anyhow errors that should be treated as internal errors
    pub async fn start_passkey_authentication(
        &self,
        repo: &mut impl RepositoryAccess,
        rng: &mut (dyn RngCore + Send),
        clock: &impl Clock,
    ) -> Result<(String, UserPasskeyChallenge)> {
        let options = PublicKeyCredentialRequestOptions::passkey(&self.rpid);

        let (server_state, client_state) = options.start_ceremony()?;

        let user_passkey_challenge = repo
            .user_passkey()
            .add_challenge(rng, clock, server_state.encode()?)
            .await?;

        Ok((
            serde_json::to_string(&client_state)?,
            user_passkey_challenge,
        ))
    }

    /// Finds the passkey and user based on the challenge response and validates
    /// that the passkey belongs to the user
    ///
    /// # Returns
    /// 1. The parsed response for use later
    /// 2. The [`User`] trying to authenticate
    /// 3. The [`UserPasskey`] used
    ///
    /// # Errors
    /// [`WebauthnError::UserHandleMissing`] if the reponse doesn't contain the
    /// user handle.
    ///
    /// [`WebauthnError::UserNotFound`] if the user wasn't found.
    ///
    /// [`WebauthnError::PasskeyNotFound`] if the passkey wasn't found.
    ///
    /// [`WebauthnError::UserMismatch`] if the passkey is tied to a different
    /// user.
    ///
    /// The rest of the anyhow errors should be treated as internal errors
    pub async fn discover_credential(
        &self,
        repo: &mut impl RepositoryAccess,
        response: String,
    ) -> Result<(Authentication, User, UserPasskey)> {
        let response = serde_json::from_str::<AuthenticationRelaxed>(&response)?.0;

        let credential_id = serde_json::to_string(&response.raw_id())?;

        let id_bytes = response
            .response()
            .user_handle()
            .ok_or(WebauthnError::UserHandleMissing)?
            .into_inner()
            .try_into()?;
        let user_id = Ulid::from_bytes(id_bytes);

        let user = repo
            .user()
            .lookup(user_id)
            .await?
            .ok_or(WebauthnError::UserNotFound)?;

        let user_passkey = repo
            .user_passkey()
            .find(&credential_id)
            .await?
            .ok_or(WebauthnError::PasskeyNotFound)?;

        if user_passkey.user_id != user.id {
            return Err(WebauthnError::UserMismatch.into());
        }

        Ok((response, user, user_passkey))
    }

    /// Validates the authentication challenge response
    ///
    /// # Errors
    /// [`WebauthnError::AuthenticationCeremonyError`] if the response from the
    /// user was invalid.
    ///
    /// The rest of the anyhow errors should be treated as internal errors
    pub async fn finish_passkey_authentication(
        &self,
        repo: &mut impl RepositoryAccess,
        clock: &impl Clock,
        user_passkey_challenge: UserPasskeyChallenge,
        response: Authentication,
        user_passkey: UserPasskey,
    ) -> Result<UserPasskey> {
        let server_state = AuthenticationServerState::decode(&user_passkey_challenge.state)?;

        let options = AuthenticationVerificationOptions::<DomainOrigin, DomainOrigin> {
            allowed_origins: &[self.get_allowed_origin()],
            client_data_json_relaxed: true,
            ..Default::default()
        };

        // Construct the correct type of user handle...
        let user_handle = UserHandle::<[u8; 16]>::decode(user_passkey.user_id.to_bytes())?;
        let user_handle = UserHandle::<&[u8]>::from(&user_handle);

        // Construct the correct type of credential ID...
        let credential_id =
            serde_json::from_str::<CredentialId<Vec<u8>>>(&user_passkey.credential_id)?;
        let credential_id = CredentialId::<&[u8]>::from(&credential_id);

        // Convert stored passkey to a usable credential
        let mut cred = AuthenticatedCredential::new(
            credential_id,
            user_handle,
            StaticState::decode(&user_passkey.static_state)?,
            DynamicState::decode(
                user_passkey
                    .dynamic_state
                    .clone()
                    .try_into()
                    .map_err(|_| anyhow::Error::msg("Failed to parse dynamic state"))?,
            )?,
        )?;

        server_state
            .verify(&self.rpid, &response, &mut cred, &options)
            .map_err(WebauthnError::from)?;

        // Update last used date and dynamic state
        let user_passkey = repo
            .user_passkey()
            .update(clock, user_passkey, cred.dynamic_state().encode()?.to_vec())
            .await?;

        // Ensure that the challenge gets marked as completed if it wasn't already
        if user_passkey_challenge.completed_at.is_none() {
            repo.user_passkey()
                .complete_challenge(clock, user_passkey_challenge)
                .await?;
        }

        Ok(user_passkey)
    }
}
