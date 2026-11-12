"use strict";
// A tiny classic script runs independently of the Owner Lab module graph.
// If a browser cannot import app.js, the operator must not stare at "Загрузка…"
// indefinitely. Do not retry provider requests or change authorization here.
(() => {
  const status = document.getElementById("status");
  if (!status) return;
  const initialText = status.textContent?.trim() ?? "";
  if (initialText !== "Загрузка…") return;
  // Diagnostics stay local and fixed-label: never record URLs, script content,
  // tokens or provider payloads, and never change the authorization path.
  let failureKind = "module-timeout";
  const bootstrapModules = new Set([
    "/app.js", "/bootstrap-context.js", "/owner-capture.js",
    "/reference-capture.js", "/provider-event-fence.js",
  ]);
  window.addEventListener("error", (event) => {
    if (status.textContent?.trim() !== initialText) return;
    if (event.target instanceof HTMLScriptElement) {
      try {
        const url = new URL(event.target.src, window.location.href);
        if (url.origin === window.location.origin && bootstrapModules.has(url.pathname)) {
          failureKind = "module-load";
        }
      } catch {
        // An invalid URL is not an authority to collect browser details.
      }
    } else if (event.target === window) {
      failureKind = "module-execution";
    }
  }, true);
  const deadlineMs = 12000;
  window.setTimeout(() => {
    // A late, legitimate bootstrap may still replace this error. Do not
    // overwrite a running or already failed application with false success.
    if (status.textContent?.trim() !== initialText) return;
    status.dataset.state = "error";
    status.dataset.bootstrapFailure = failureKind;
    status.textContent = "Интерфейс не загрузился. Проверьте подключение и обновите страницу.";
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "Повторить загрузку";
    retry.addEventListener("click", () => window.location.reload());
    status.appendChild(document.createTextNode(" "));
    status.appendChild(retry);
  }, deadlineMs);
})();
