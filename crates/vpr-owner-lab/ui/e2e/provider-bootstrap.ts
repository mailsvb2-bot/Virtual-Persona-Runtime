import type { Page } from "@playwright/test";

/**
 * Arms provider integration without requiring a production-only test event or a late
 * Playwright RPC. The harness observes the real DOM control and performs the same click
 * only after application state has made it actionable.
 */
export const installProviderAutoConnect = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    const arm = (): void => {
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

      const tryConnect = (): boolean => {
        consent.checked = true;
        if (connect.disabled) return false;
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
