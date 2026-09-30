import type { Page } from "@playwright/test";

/**
 * Provider-integration journeys do not own human click/actionability proof.
 * Browser-contract E2E covers the visible consent and Connect controls.
 *
 * This test-only init script waits for the production UI to finish its real bootstrap,
 * then sets the real consent control and clicks the real Connect button inside the page.
 * Production exposes no test hook or alternate connect path.
 */
export const installProviderAutoConnect = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    const arm = (): void => {
      const root = document.documentElement;
      const connect = document.getElementById("connect");
      const consent = document.getElementById("consent");
      if (!(connect instanceof HTMLButtonElement) || !(consent instanceof HTMLInputElement)) {
        root.dataset.vprProviderAutoConnect = "controls-missing";
        return;
      }

      const tryConnect = (): boolean => {
        if (connect.disabled) return false;
        consent.checked = true;
        root.dataset.vprProviderAutoConnect = "clicked";
        connect.click();
        return true;
      };

      if (tryConnect()) return;
      const observer = new MutationObserver(() => {
        if (!tryConnect()) return;
        observer.disconnect();
      });
      observer.observe(connect, { attributes: true, attributeFilter: ["disabled"] });
    };

    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", arm, { once: true });
    } else {
      arm();
    }
  });
};
