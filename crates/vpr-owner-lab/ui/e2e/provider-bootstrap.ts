import type { Page } from "@playwright/test";

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
