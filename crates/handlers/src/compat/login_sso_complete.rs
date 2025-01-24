// Copyright 2024 New Vector Ltd.
// Copyright 2022-2024 The Matrix.org Foundation C.I.C.
//
// SPDX-License-Identifier: AGPL-3.0-only
// Please see LICENSE in the repository root for full details.

use std::collections::HashMap;

use anyhow::Context;
use axum::{
    extract::{Form, Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::Duration;
use mas_axum_utils::{
    cookies::CookieJar,
    csrf::{CsrfExt, ProtectedForm},
    FancyError, SessionInfoExt,
};
use mas_data_model::Device;
use mas_matrix::BoxHomeserverConnection;
use mas_router::{CompatLoginSsoAction, UrlBuilder};
use mas_storage::{
    compat::{CompatSessionRepository, CompatSsoLoginRepository},
    BoxClock, BoxRepository, BoxRng, Clock, RepositoryAccess,
};
use mas_templates::{CompatSsoContext, ErrorContext, TemplateContext, Templates};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::PreferredLanguage;

#[derive(Serialize)]
struct AllParams<'s> {
    #[serde(flatten)]
    existing_params: HashMap<&'s str, &'s str>,

    #[serde(rename = "loginToken")]
    login_token: &'s str,
}

#[derive(Debug, Deserialize)]
pub struct Params {
    action: Option<CompatLoginSsoAction>,
}

#[tracing::instrument(
    name = "handlers.compat.login_sso_complete.get",
    fields(compat_sso_login.id = %id),
    skip_all,
    err,
)]
pub async fn get(
    PreferredLanguage(locale): PreferredLanguage,
    mut rng: BoxRng,
    clock: BoxClock,
    mut repo: BoxRepository,
    State(templates): State<Templates>,
    State(url_builder): State<UrlBuilder>,
    cookie_jar: CookieJar,
    Path(id): Path<Ulid>,
    Query(params): Query<Params>,
) -> Result<Response, FancyError> {
    let (session_info, cookie_jar) = cookie_jar.session_info();
    let (csrf_token, cookie_jar) = cookie_jar.csrf_token(&clock, &mut rng);

    let maybe_session = session_info.load_session(&mut repo).await?;

    let Some(session) = maybe_session else {
        // If there is no session, redirect to the login or register screen
        let url = match params.action {
            Some(CompatLoginSsoAction::Register) => {
                url_builder.redirect(&mas_router::Register::and_continue_compat_sso_login(id))
            }
            Some(CompatLoginSsoAction::Login) | None => {
                url_builder.redirect(&mas_router::Login::and_continue_compat_sso_login(id))
            }
        };

        return Ok((cookie_jar, url).into_response());
    };

    let login = repo
        .compat_sso_login()
        .lookup(id)
        .await?
        .context("Could not find compat SSO login")?;

    // Bail out if that login session is more than 30min old
    if clock.now() > login.created_at + Duration::microseconds(30 * 60 * 1000 * 1000) {
        let ctx = ErrorContext::new()
            .with_code("compat_sso_login_expired")
            .with_description("This login session expired.".to_owned())
            .with_language(&locale);

        let content = templates.render_error(&ctx)?;
        return Ok((cookie_jar, Html(content)).into_response());
    }

    let ctx = CompatSsoContext::new(login)
        .with_session(session)
        .with_csrf(csrf_token.form_value())
        .with_language(locale);

    let content = templates.render_sso_login(&ctx)?;

    Ok((cookie_jar, Html(content)).into_response())
}

#[tracing::instrument(
    name = "handlers.compat.login_sso_complete.post",
    fields(compat_sso_login.id = %id),
    skip_all,
    err,
)]
pub async fn post(
    mut rng: BoxRng,
    clock: BoxClock,
    mut repo: BoxRepository,
    PreferredLanguage(locale): PreferredLanguage,
    State(templates): State<Templates>,
    State(url_builder): State<UrlBuilder>,
    State(homeserver): State<BoxHomeserverConnection>,
    cookie_jar: CookieJar,
    Path(id): Path<Ulid>,
    Query(params): Query<Params>,
    Form(form): Form<ProtectedForm<()>>,
) -> Result<Response, FancyError> {
    let (session_info, cookie_jar) = cookie_jar.session_info();
    cookie_jar.verify_form(&clock, form)?;

    let maybe_session = session_info.load_session(&mut repo).await?;

    let Some(session) = maybe_session else {
        // If there is no session, redirect to the login or register screen
        let url = match params.action {
            Some(CompatLoginSsoAction::Register) => {
                url_builder.redirect(&mas_router::Register::and_continue_compat_sso_login(id))
            }
            Some(CompatLoginSsoAction::Login) | None => {
                url_builder.redirect(&mas_router::Login::and_continue_compat_sso_login(id))
            }
        };

        return Ok((cookie_jar, url).into_response());
    };

    let login = repo
        .compat_sso_login()
        .lookup(id)
        .await?
        .context("Could not find compat SSO login")?;

    // Bail out if that login session is more than 30min old
    if clock.now() > login.created_at + Duration::microseconds(30 * 60 * 1000 * 1000) {
        let ctx = ErrorContext::new()
            .with_code("compat_sso_login_expired")
            .with_description("This login session expired.".to_owned())
            .with_language(&locale);

        let content = templates.render_error(&ctx)?;
        return Ok((cookie_jar, Html(content)).into_response());
    }

    let redirect_uri = {
        let mut redirect_uri = login.redirect_uri.clone();
        let existing_params = redirect_uri
            .query()
            .map(serde_urlencoded::from_str)
            .transpose()?
            .unwrap_or_default();

        let params = AllParams {
            existing_params,
            login_token: &login.login_token,
        };
        let query = serde_urlencoded::to_string(params)?;
        redirect_uri.set_query(Some(&query));
        redirect_uri
    };

    // Lock the user sync to make sure we don't get into a race condition
    repo.user().acquire_lock_for_sync(&session.user).await?;

    let device = Device::generate(&mut rng);
    let mxid = homeserver.mxid(&session.user.username);
    homeserver
        .create_device(&mxid, device.as_str())
        .await
        .context("Failed to provision device")?;

    let compat_session = repo
        .compat_session()
        .add(
            &mut rng,
            &clock,
            &session.user,
            device,
            Some(&session),
            false,
        )
        .await?;

    repo.compat_sso_login()
        .fulfill(&clock, login, &compat_session)
        .await?;

    repo.save().await?;

    Ok((cookie_jar, Redirect::to(redirect_uri.as_str())).into_response())
}
