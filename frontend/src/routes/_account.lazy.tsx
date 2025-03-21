// Copyright 2024, 2025 New Vector Ltd.
// Copyright 2024 The Matrix.org Foundation C.I.C.
//
// SPDX-License-Identifier: AGPL-3.0-only
// Please see LICENSE in the repository root for full details.

import { Outlet, createLazyFileRoute, notFound } from "@tanstack/react-router";
import { Heading } from "@vector-im/compound-web";
import { useTranslation } from "react-i18next";
import Layout from "../components/Layout";
import NavBar from "../components/NavBar";
import NavItem from "../components/NavItem";
import UserGreeting from "../components/UserGreeting";

import { useSuspenseQuery } from "@tanstack/react-query";
import { query } from "./_account";

export const Route = createLazyFileRoute("/_account")({
  component: Account,
});

function Account(): React.ReactElement {
  const { t } = useTranslation();
  const result = useSuspenseQuery(query);
  const viewer = result.data.viewer;
  if (viewer?.__typename !== "User") throw notFound();
  const siteConfig = result.data.siteConfig;

  return (
    <Layout wide>
      <div className="flex flex-col gap-10">
        <Heading size="md" weight="semibold">
          {t("frontend.account.title")}
        </Heading>

        <div className="flex flex-col gap-4">
          <UserGreeting user={viewer} siteConfig={siteConfig} />

          <NavBar>
            <NavItem to="/">{t("frontend.nav.settings")}</NavItem>
            <NavItem to="/sessions">{t("frontend.nav.devices")}</NavItem>
          </NavBar>
        </div>
      </div>

      <Outlet />
    </Layout>
  );
}
