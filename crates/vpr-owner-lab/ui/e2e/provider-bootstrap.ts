import type { Page } from "@playwright/test";

/**
 * Arms provider-integration journeys before navigation.
 *
 * Browser-contract E2E owns human click/actionability proof. Provider journeys only need the
 * canonical connect event to happen after the app has completed bootstrap. Installing this hook
 * at document creation avoids a late Playwright/CDP mutation racing the live renderer.
 */
export const installProviderAutoConnect = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    document.addEventListener("DOMContentLoaded", () => {
      const consent = document.getElementById("consent");
      if (consent instanceof HTMLInputElement) consent.checked = true;
    }, { once: true });

    window.addEventListener("vpr:bootstrap-ready", () => {
      const root = document.documentElement;
      const connect = document.getElementById("connect");
      const consent = document.getElementById("consent");

      if (!(connect instanceof HTMLButtonElement)) {
        root.dataset.vprProviderAutoConnect = "CONNECT_CONTROL_MISSING";
        return;
      }
      if (!(consent instanceof HTMLInputElement) || !consent.checked) {
        root.dataset.vprProviderAutoConnect = "CONNECT_CONSENT_NOT_PRESEEDED";
        return;
      }
      if (connect.disabled) {
        root.dataset.vprProviderAutoConnect = "CONNECT_CONTROL_DISABLED";
        return;
      }

      root.dataset.vprProviderAutoConnect = "clicked";
      connect.click();
    }, { once: true });
  });
};
