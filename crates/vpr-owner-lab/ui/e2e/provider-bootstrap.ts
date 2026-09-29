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
    window.addEventListener("vpr:bootstrap-ready", () => {
      const root = document.documentElement;
      const connect = document.getElementById("connect");
      const consent = document.getElementById("consent");

      if (!(connect instanceof HTMLButtonElement)) {
        root.dataset.vprProviderAutoConnect = "CONNECT_CONTROL_MISSING";
        return;
      }
      if (!(consent instanceof HTMLInputElement)) {
        root.dataset.vprProviderAutoConnect = "CONNECT_CONSENT_MISSING";
        return;
      }
      // Provider E2E owns the provider/backend path, not the human consent-click proof.
      // Set consent at the exact bootstrap-ready boundary so DOM parsing/order cannot race it.
      consent.checked = true;
      if (!consent.checked) {
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
