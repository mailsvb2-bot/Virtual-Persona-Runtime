import type { Page } from "@playwright/test";

/**
 * Arms the real Owner Lab consent/connect controls before navigation.
 *
 * Media-provider E2E must not depend on post-navigation CDP actionability while the
 * browser is bringing up synthetic media primitives. This harness stays entirely in
 * the test document: it waits for the application's visible ready state, checks the
 * real consent checkbox, and invokes the real Connect button. Production exposes no
 * test event, callback, or alternate connect path.
 */
export const installProviderAutoConnect = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    const arm = (): void => {
      const root = document.documentElement;
      const connect = document.getElementById("connect");
      const consent = document.getElementById("consent");
      const status = document.getElementById("status");

      if (
        !(connect instanceof HTMLButtonElement)
        || !(consent instanceof HTMLInputElement)
        || !(status instanceof HTMLElement)
      ) {
        root.dataset.vprProviderAutoConnect = "controls-missing";
        return;
      }

      const tryConnect = (): boolean => {
        const applicationReady = status.textContent?.includes("Готов к подключению") === true;
        if (!applicationReady || connect.disabled) return false;
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
      observer.observe(status, { childList: true, subtree: true, characterData: true });
    };

    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", arm, { once: true });
    } else {
      arm();
    }
  });
};
