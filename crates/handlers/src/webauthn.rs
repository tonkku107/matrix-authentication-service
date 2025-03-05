// Copyright 2025 New Vector Ltd.
//
// SPDX-License-Identifier: AGPL-3.0-only
// Please see LICENSE in the repository root for full details.

use anyhow::{Context, Result};
use url::Url;
use webauthn_rs::{Webauthn, WebauthnBuilder};

/// Builds a webauthn instance.
///
/// # Errors
/// If the public base url doesn't have a host or the webauthn configuration is invalid
pub fn get_webauthn(public_base: &Url) -> Result<Webauthn> {
    Ok(WebauthnBuilder::new(
        public_base
            .host_str()
            .context("Public base doesn't have a host")?,
        public_base,
    )?
    .build()?)
}
